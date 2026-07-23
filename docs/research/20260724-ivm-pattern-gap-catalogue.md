# IVM pattern gap catalogue — what the field has that smelt's registry doesn't

**Date**: 2026-07-24
**Status**: survey — no decisions. Input for future Future-Extensions entries and registry
growth, following the recognition of the keyed-succession (SCD2) pattern in
`docs/research/20260723-scd2-succession-pattern.md`.

## Question and method

`docs/specs/incremental_models.md` fixes a registry of maintenance machinery: the read×write
2×2 (fold-a-delta, read-modify-write region, column-scoped re-derivation, recompute-a-region),
an open physical write-pattern set (region DELETE+INSERT, keyed/column-scoped MERGE, in-place
UPDATE, full rebuild; contemplated: partition swap, CoW/MoR, `NOT MATCHED BY SOURCE` prune,
staged upsert, FD-targeted UPDATE), the four-rung combiner-algebra ladder, and the sketched
keyed-succession patch. The question: **what distinct maintenance/update patterns exist in IVM
research and production engines that this registry misses?**

Method: a fan-out web survey (classic IVM theory, higher-order/algebraic IVM, streaming
engines, warehouse refresh strategies, lakehouse write patterns, special structures) over 29
primary sources, with per-claim adversarial verification of the top claims. Claims marked
**[verified]** survived a 3-vote adversarial check against the source; unmarked claims are
single-extraction (quote-backed but not adversarially re-checked). This complements
`docs/research/20260703-model-updates.md` Part 12, which validated the *rejection* catalogue;
this note is about *mechanisms* smelt could add.

Entries are grouped by which part of the registry they would extend. Each states what it
maintains, the mechanism, the source, and a fit assessment against smelt's equivalence
invariant (`incremental_state(S) == full_refresh(inputs ∈ S)`).

---

## A. Combiner-ladder extensions

### A1. Per-group targeted recompute as the retraction escape hatch (high value)

**What**: non-invertible combiners (`MIN`/`MAX`/`BOOL_*`) under deletes/corrections, without a
whole-model full refresh.
**Mechanism**: when a delta retracts a contribution a non-invertible combiner cannot un-see,
recompute *only the affected groups* from replayable input — located by semijoining the delta's
keys against the source — instead of refusing the whole model. pg_ivm: "If the old min(x) or
max(x) is deleted from the view, it needs recomputing the new value from base tables"
**[verified]** ([PostgreSQL IVM wiki](https://wiki.postgresql.org/wiki/Incremental_View_Maintenance)).
Oracle does the same for MIN/MAX under deletes; Snowflake maintains GROUP BY by "recomputing
the aggregate only for grouping keys that contain changes" and documents the cost threshold
(efficient when changes touch ⪅5% of keys); Databricks Enzyme formalises it as
`Δ(G(T)) = π−(G(T ⋉ ΔT)) + π+(G(T′ ⋉ ΔT))` and applies delete-keys-then-append-fresh via MERGE
([Snowflake incremental operators](https://docs.snowflake.com/en/user-guide/dynamic-tables-performance-incremental-operators),
[Enzyme, SIGMOD '26](https://arxiv.org/pdf/2603.27775)).
**Fit**: direct. It is the column-scoped re-derivation corner narrowed from "the region's full
upstream input" to "the affected keys' full upstream input" — read full-input-per-key, write
keyed. The equivalence invariant holds because the recompute *is* the full-refresh expression
per group. Today smelt's ladder rung 3 says non-invertible + retraction ⇒ "cannot be
reprocessed without a full refresh"; every surveyed engine that publishes its mechanism uses
per-group recompute instead. This is arguably the single highest-value missing technique: it
converts smelt's harshest refusal into a bounded repair, using only machinery smelt already
has (replayable input, key location, keyed MERGE).

### A2. Derivation-count-annotated state (the counting algorithm)

**What**: duplicate-eliminating shapes under deletes — `DISTINCT`, distinct `UNION`,
`EXISTS`/semijoin projections — currently outside every smelt corner.
**Mechanism**: store, per output row, a hidden count of its *derivations* (not a value→count
domain multiset); deltas carry signed counts; a row is deleted when its count reaches zero.
Gupta–Mumick–Subrahmanian: "we store only the number of derivations, not the derivations
themselves", and the algorithm is optimal — it computes exactly the inserted/deleted view
tuples **[verified]** ([SIGMOD '93](https://dl.acm.org/doi/10.1145/170035.170066),
[Gupta & Mumick '95 survey](https://www.academia.edu/4609781/Maintenance_of_Materialized_Views_Problems_Techniques_and_Applications)).
pg_ivm implements it as a hidden `__ivm_count__` column incremented on duplicate insert
**[verified]**.
**Fit**: good. State is `O(|output|)` (one int per stored row), unlike rung 4's
`O(active domain)` multiset — so it needs no opt-in budget. It is structurally a decomposed
monoid (hidden state + presentation view that drops the count), which is exactly smelt's rung
2 shape applied to row *existence* rather than a value column. Would let a keyed model project
`DISTINCT` or `EXISTS` semantics under retraction-bearing feeds.

### A3. Ordered-semigroup folds: ordering substitutes for commutativity

**What**: non-commutative combiners — last-value/replace, `FIRST_VALUE`, listagg — which
smelt's rung 1 (commutative monoid) excludes.
**Mechanism**: Google Mesa requires its table-declared aggregation function to be
**associative but not commutative**, because deltas are always merged in strict version order;
it runs production tables with `F(v0,v1) = v1` (last-write-wins replace). Retractions are
"negative facts" — inverse rows folded through the same function — whose correctness rests on
the total version order guaranteeing a retraction never applies before the fact it retracts
([Mesa, VLDB '14](https://static.googleusercontent.com/media/research.google.com/en//pubs/archive/42851.pdf)).
Hudi's `COMMIT_TIME_ORDERING` / `EVENT_TIME_ORDERING` merge modes and Paimon's sequence fields
are the lakehouse form: latest-by-declared-ordering-field wins, absorbing out-of-order arrivals
([Hudi record merger](https://hudi.apache.org/docs/next/record_merger/),
[Paimon merge engines](https://paimon.apache.org/docs/0.8/primary-key-table/merge-engine/)).
**Fit**: direct, and smelt half-owns it already: the key grain's snapshot-reconcile is
latest-wins, and the monotone clock supplies the order. The gap is that the *ladder* doesn't
name the rung — "associative + totally-ordered application" sits between rung 1 and
unmaintainable, and would admit `MAX_BY`/`ARG_MAX`-style latest-attribute columns and
event-time-ordered upserts as derived verdicts rather than special cases. Note the late-event
interaction: ordering by *event time* (Hudi `EVENT_TIME_ORDERING`) keeps the fold
replay-order-independent, which is what the equivalence invariant needs; ordering by
commit/arrival time does not.

### A4. Ring-typed payloads and view trees (F-IVM / DBToaster)

**What**: analytics beyond scalar aggregates — covariance matrices / linear-regression state,
mutual information — maintained with one algorithm.
**Mechanism**: F-IVM generalises the combiner to an arbitrary **ring** with key/payload
separation (e.g. the (count, sum-vector, quadratic-matrix) covariance ring), and maintains a
**view tree** of partial-aggregate views: an update folds through only the leaf-to-root path
([F-IVM](https://arxiv.org/pdf/2303.08583)). DBToaster's viewlet transform is the recursive
extreme: materialize the delta queries themselves as views, maintain them with higher-order
deltas; each delta strictly reduces query degree, so recursion terminates, and for a large SQL
fragment refresh reduces to pure summation with no join evaluation **[verified]**
([DBToaster, PVLDB '12](http://vldb.org/pvldb/vol5/p968_yanifahmad_vldb2012.pdf)). Both papers
note full materialization isn't always best — DBToaster uses a cost model to trade auxiliary
materialization against lazy re-evaluation.
**Fit**: the ring generalisation is a clean widening of rung 2 (richer state + presentation
map) if demand for regression-style columns appears. The view-tree/higher-order idea maps to
smelt's *cross-model* ambitions rather than per-model maintenance: auxiliary partial-aggregate
state between a model and its sources is a planner-materialized helper model. Large machinery;
justified only by join-heavy aggregate demand.

### A5. Bounded-state ranked-prefix maintenance (top-k)

**What**: `QUALIFY ROW_NUMBER() OVER (PARTITION BY k ORDER BY v) <= N` models — the
top-N-per-key shape, today refused with the rest of window functions.
**Mechanism**: RisingWave *recognises the syntactic pattern* (ranking function + rank-range
filter) and switches to a dedicated operator retaining only the top-N rows per partition,
versus a general over-window operator keeping full per-partition state
([RisingWave Top-N](https://docs.risingwave.com/processing/sql/top-n-by-group)). Same
recognition-over-declaration philosophy as the keyed-succession sketch.
**Fit**: split verdict. Append-only feeds: the retained top-N *is* sufficient state and the
fold is a bounded-multiset monoid — fits the ladder directly, a natural sibling classifier to
the succession pattern. Retraction-bearing feeds: evicted rows may need to re-enter, so the
bounded state is insufficient without either the A1 escape hatch (recompute affected keys) or
a widened candidate buffer. Worth cataloguing as a second window-function family the
succession classifier's machinery (walk vocabulary for ORDER BY semantics) would enable.

---

## B. Technique corners and read-side patterns

### B1. Outer-join view maintenance via primary + clean-up delta

**What**: SPOJ views (selection/projection/inner **and outer** joins) maintained
incrementally — null-extended rows repaired when matches appear or disappear.
**Mechanism**: rewrite into join-disjunctive normal form (a minimum union of inner-join terms
whose net contributions are independent), apply a **primary delta** exactly like inner-join
maintenance, then a **secondary "clean-up" delta**: inserts can newly subsume previously
visible null-extended tuples (requiring deletes), deletes can re-orphan tuples (requiring
inserts), located via a subsumption graph over terms **[verified]**
([Larson & Zhou](http://www.cs.columbia.edu/~jrzhou/pub/OJViewMaintenance.pdf)). Production
confirmations: Snowflake maintains outer-join views incrementally but only under equality
predicates, using inner-join delta logic plus `NOT EXISTS` anti-joins for the null-padded
rows; Oracle gates it behind unique constraints on the inner table's join columns.
**Fit**: plausible and structurally familiar — the secondary delta is a generalisation of the
succession pattern's predecessor patch ("a write to row X obliges a patch to related row Y,
locatable from stored state"). The write patterns are ones smelt has (keyed MERGE + targeted
DELETE/INSERT). What's missing is the admission analysis: a null-extension obligation per
outer-join edge, discharged when the clean-up rows are locatable and bounded. Candidate
Future-Extensions entry when outer joins over mutable dimensions show demand.

### B2. Self-maintainability and co-materialized auxiliary state

**What**: maintenance that never re-reads upstream inputs — using only the delta, the view's
own content, and declared constraints (or extra materialized side-views).
**Mechanism**: Gupta–Mumick name the class: some views are provably not maintainable from the
view alone under deletions, but become maintainable given any of the base relation, a key
constraint, or stored derivation counts; SPJ views are often self-maintainable for deletions
when join keys are preserved in the output, never for insertions when joining ≥2 relations
**[verified]**. The follow-on literature ("Making Views Self-Maintainable for Data
Warehousing") derives a *set of auxiliary views* co-materialized so the target plus auxiliaries
are jointly maintainable from deltas alone — with the auxiliary set chosen per update workload
([survey](https://www.academia.edu/4609781/Maintenance_of_Materialized_Views_Problems_Techniques_and_Applications)).
**Fit**: this is a **plan axis smelt doesn't have** rather than a technique: per cell, *what
the maintenance read is allowed to touch* (delta only / delta + own state / delta + upstream).
smelt's admission machinery answers "is the read bounded" but not "can the upstream read be
eliminated given the declared identity and preserved keys". Payoff: mutation cells that today
probe an upstream dimension could sometimes be served from the output's own columns. The
Gupta–Mumick four-dimension taxonomy (information available × modification × language ×
instance) is also a useful lens on the registry itself — smelt's registry is strong on
modification/language, thin on the information dimension.

### B3. ID-based diffs (i-diffs) — compact key-addressed deltas

**What**: propagating a small dimension change to many output rows without recomputing or even
enumerating full output tuples.
**Mechanism**: idIVM computes diffs identified by base-table keys, where one i-diff row
represents modifications to many view rows, applied as a predicate-targeted UPDATE keyed on a
subset of the view's ID attributes, setting only changed non-ID attributes; propagating a
single-table change skips joins with unaffected base tables entirely **[verified]**
([idIVM, SIGMOD '15](https://dl.acm.org/doi/10.1145/2723372.2750546)).
**Fit**: this is the literature anchor for the registry's contemplated "UPDATE that locates
rows through the join key rather than the output's `unique_key`" (spec §"The write-pattern set
is open"). The paper supplies what the spec entry says the pattern must declare: base-table
keys as the required contract fact, and the FD from join key to repaired columns as the
equivalence obligation. Also validates smelt's column-group factoring — i-diffs are per-column
deltas in substance.

### B4. Delta-shape classification as an explicit plan input

**What**: choosing different maintenance code paths by the *shape of the delta* (append-only
vs mixed DML), not just the shape of the query.
**Mechanism**: Oracle's fast refresh takes distinct optimized paths for insert-only vs mixed
deltas and degrades an aggregate MV without `COUNT(*)` to an "insert-only materialized view"
(only append deltas foldable; other DML forces complete refresh)
([Oracle refresh docs](https://docs.oracle.com/database/122/DWHSG/refreshing-materialized-views.htm)).
**Fit**: validation more than gap — smelt's faithful-fold conditions (source posture ×
combiner algebra) already encode delta shape. The addition worth taking: the *same cell* can
carry two techniques selected per run by observed delta shape (append-only run → fold; a run
whose delta bears retractions → A1's group recompute), which today smelt expresses only as a
static refusal.

---

## C. Physical write patterns (registry candidates)

### C1. Diff-then-patch (reconciliation write) — recompute without region overwrite

**What**: any recompute cell — writing only the rows that actually changed.
**Mechanism**: Materialize's "self-correcting sink": compute the desired output, continuously
diff it against the persisted output collection, and write only the correcting delta, so
stored state converges to desired state
([Materialize blog](https://materialize.com/blog/self-correcting-materialized-views/)). The
batch form is: recompute the region into a staging relation, anti-join both directions against
the stored region, emit targeted DELETE/UPDATE/INSERT instead of DELETE+INSERT of the whole
region.
**Fit**: direct registry candidate. Required contract facts: row identity within the region
(or full-row comparison). Equivalence is trivial (the end state is definitionally the
recompute's output); what it changes is write *volume*, which matters on CoW table formats
where rewritten files are the cost, and for downstream CDC consumers who see only true
changes. Notably it composes with smelt's change-suppressed column-scoped MERGE — same idea,
generalised from "suppress no-op column writes" to "suppress no-op row writes under
recompute". Also the natural mechanism for the spec's open **changed-column definition
trigger**: Materialize uses exactly this to swap a view's definition in place, emitting only
the old-vs-new output diff, without rebuilding downstream consumers.

### C2. Shadow-build-and-swap (out-of-place refresh)

**What**: any refresh, with the old output fully queryable until an atomic switch.
**Mechanism**: Oracle out-of-place refresh builds the new/affected portions into separate
outside tables (indexes included), then completes by table switch or partition exchange;
applies to fast, PCT, and complete refresh
([Oracle](https://docs.oracle.com/database/122/DWHSG/refreshing-materialized-views.htm)).
**Fit**: generalises the contemplated partition-swap entry from "swap a partition" to "swap
the relation", motivated by availability during refresh rather than by addressing. Fits the
open registry cleanly (declares: backend swap/exchange capability; equivalence unchanged —
it's the same statement writing to a staging target). SQLMesh's environment/virtual-layer swap
is the transformation-framework relative of the same pattern.

### C3. Append-delta + asynchronous compaction (merge-on-read as a maintenance pattern)

**What**: keyed maintained state where the *write* is always a cheap sorted append and the
combiner fold is deferred to compaction/read.
**Mechanism**: Napa/Mesa maintain tables and views as LSM merge-forests of versioned sorted
deltas; compaction is an aggregating merge-sort (merge + combine over key-sorted runs);
queries merge a bounded spanning set of deltas at read time. Mesa's multi-level compaction
policy (base + cumulative levels + singletons) is an explicit knob trading update latency vs
query cost vs storage
([Napa, VLDB '21](http://www.vldb.org/pvldb/vol14/p2986-sankaranarayanan.pdf),
[Mesa, VLDB '14](https://static.googleusercontent.com/media/research.google.com/en//pubs/archive/42851.pdf)).
Materialize's arrangement traces are the same structure in memory; Hudi MoR defers the merge
function to compaction and query time.
**Fit**: as a smelt-*emitted* pattern, poor — it's a storage engine's job. As a
**backend-provided pattern admitted through the capability registry**, strong: on
Paimon/Hudi-class backends, a fold cell's physical write can be "append the delta rows; the
table's declared merge engine performs the fold" (see C4). The registry's backend-relative
design anticipates exactly this. Napa also contributes two transferable ideas: **view deltas
computed by applying the view's SQL to base deltas via an ordinary batch SQL engine** (which
is precisely smelt's posture, an external validation), and **key-prefix alignment classes** —
maintenance cost classed by whether view sort/partition keys share a prefix with the input's
(full prefix → streaming merge, no shared prefix → full re-sort) — a cost-model input smelt's
partition-alignment analysis could adopt.

### C4. Storage-merge engines: per-field combiner folds and column-slice assembly

**What**: keyed tables whose per-column fold happens inside the table format's merge path.
**Mechanism**: Paimon's **aggregation merge engine** attaches an aggregate function per field
(sum, product, min/max, listagg, collect, merge_map, nested_update, HLL/theta sketches,
roaring bitmaps…), folded at compaction; retraction support is per-function, with exactly the
invertible subset (sum, product, count, collect, merge_map, nested_update, last_value…)
handling `UPDATE_BEFORE`/`DELETE` and the rest requiring `ignore-retract`
([Paimon merge engines](https://paimon.apache.org/docs/0.8/primary-key-table/merge-engine/)).
The **partial-update merge engine** assembles one row per key from multiple writers each
supplying a column subset, null never overwriting non-null, with **sequence groups** giving
each column group its own ordering field to resolve out-of-order multi-stream writes
([Paimon partial-update](https://paimon.apache.org/docs/master/primary-key-table/merge-engine/partial-update/)).
Its **first-row engine** is insert-if-absent/first-writer-wins. Hudi's pluggable
`HoodieRecordMerger` requires the custom merge be **associative** so deferred re-application
at compaction/query time is consistent. Databricks AUTO CDC accepts partial-update change
records merged into keyed state
([DLT CDC](https://learn.microsoft.com/en-us/azure/databricks/ldp/cdc)).
**Fit**: strong independent convergence on smelt's own vocabulary — per-column-group
combiners, an invertibility ladder, write-once/first-wins columns (the accumulating-snapshot
milestone family), and ordering metadata per column group. Two takeaways: (1) these are
admissible backend write patterns (a smelt fold cell lowered to "declare the merge engine,
append deltas") once the capability registry knows them; (2) Paimon's *per-column-group
sequence fields* are a design precedent for smelt's multi-source keyed models — today smelt's
column groups are mutation-sensitivity partitions; Paimon shows the same partition carrying
per-group *ordering* metadata to absorb out-of-order multi-writer updates.

### C5. Changelog manufacture from state (bounded, versioned diffing)

**What**: producing a retraction-bearing downstream delta when the input/output pair doesn't
naturally carry one.
**Mechanism**: Paimon's `lookup` and `full-compaction` changelog producers manufacture a full
changelog (old and new values) during compaction — by point lookup of prior values, or by
diffing successive full-compaction results — because the raw merged view exposes only net
key-level changes without old values
([Paimon](https://paimon.apache.org/docs/master/)). Oracle's materialized view logs are the
classical relative: a per-base-table change-capture side structure feeding fast refresh, and
pg_ivm's unimplemented "deferred maintenance" names the same need **[verified]**.
**Fit**: directly relevant to smelt's **graph layer**, not to per-model maintenance: when a
maintained keyed model feeds another incremental model, the consumer needs the producer's
delta *with retractions*. smelt's observed-delta recording (spec §Known Divergences) is the
nascent form. The state-diff mechanism is admissible under smelt's replay rules where
snapshot-diff SCD2 is not, because the diff is between two *committed processed-input states*
(version-anchored), not between wall-clock scans — the boundary timestamps are run-set facts,
not run-clock facts. Worth stating that distinction explicitly if this is ever built.

---

## D. Plan dimensions, triggers, and scheduling

### D1. Deferred maintenance with an explicit freshness barrier

**What**: decoupling ingestion/landing from maintenance while keeping every query consistent.
**Mechanism**: Napa's **Queryable Timestamp**: queries see only data up to QT; QT advances
only when unmerged-delta count per table/view is under a bound, so ingestion, compaction, and
view maintenance run asynchronously while base tables and all views stay mutually consistent
at QT ([Napa](http://www.vldb.org/pvldb/vol14/p2986-sankaranarayanan.pdf)).
**Fit**: smelt's per-run pull model already has the ingredients (`covered_intervals`, the
reconciliation ledger, per-column settle bounds); QT is the *composed, graph-level* form — a
single monotone "consistent through" watermark over a project. A candidate future surface for
multi-model consistency guarantees (`smelt build` already batches; the QT idea is exposing the
consistent frontier as a queryable fact), not a per-model technique.

### D2. Metadata-operation triggers (partition lineage / PCT)

**What**: maintenance driven by partition-level DDL on sources — drop/exchange/load-partition —
rather than row deltas.
**Mechanism**: Oracle Partition Change Tracking derives which MV regions are stale from
partition maintenance operations on detail tables and recomputes only those; it is the *only*
fast-refresh route after such operations, and needs no MV log
([Oracle](https://docs.oracle.com/database/122/DWHSG/refreshing-materialized-views.htm)).
**Fit**: smelt's trigger axis (creation / mutation / definition change / backfill) has no
"source partition restructured" class; a partition exchange upstream is currently
indistinguishable from bulk row churn. On lakehouse backends (REPLACE PARTITION, branch
swaps), recognising the metadata operation as its own changed-input with a region-recompute
cell would be cheaper and more honest than treating it as a giant delta. Natural partner to
the contemplated partition-swap write pattern (the read-side mirror of it).

### D3. Per-refresh adaptive technique selection

**What**: choosing between admissible techniques (and full rebuild) per run, by observed cost.
**Mechanism**: Oracle `FORCE` (try fast, fall back to complete); Snowflake `ADAPTIVE` mode
(incremental by default, auto-reinitialize on detected bulk operations — versus `AUTO`, which
resolves once at creation); Enzyme selects per refresh among incremental techniques, partition
overwrite, and full recompute using a cost model over historical execution profiles of
structurally similar refreshes; DBToaster's materialize-vs-recompute heuristics are the same
decision inside one query ([Enzyme](https://arxiv.org/pdf/2603.27775)).
**Fit**: validation of smelt's "validator, not chooser" split — every one of these systems
chooses only among *equivalence-preserving* options, which is precisely the interchangeability
rule's licence. The transferable content is for smelt's unbuilt cost model: (a) delta-fraction
thresholds (Snowflake's ~5%-of-keys guidance) as the fold-vs-recompute pivot; (b) historical
per-cell run profiles as the cost estimator, which smelt's usage/state layer could record
cheaply. No new surface needed.

### D4. Temporal-filter (rolling-window) maintenance — rows that change with no input delta

**What**: views filtered by a moving now — `WHERE ts > current_timestamp - INTERVAL 7 DAY`.
**Mechanism**: Enzyme has a dedicated delta rule: remove rows falling out of the window, add
rows entering it, pass source deltas through the current predicate
([Enzyme](https://arxiv.org/pdf/2603.27775)); Snowflake admits `CURRENT_TIMESTAMP`-family
functions only in `WHERE`/`HAVING`/`QUALIFY` (clause-position-scoped determinism)
([Snowflake supported queries](https://docs.snowflake.com/en/user-guide/dynamic-tables/supported-queries)).
**Fit**: conflicts with smelt's invariant as stated — output depending on the run clock is not
replayable, which is why smelt rejects non-determinism. The principled smelt form would anchor
the window to a *processed-input fact* (the driving source's high-watermark) rather than wall
clock, making the state a pure function of `S` again. Worth an Open Questions entry only if
rolling-window marts become a demanded shape; the current "write the filter downstream or
rebuild" answer is defensible.

### D5. Incremental-over-full-refresh composability constraint

**What**: chaining an incremental model downstream of a full-rebuilt one.
**Mechanism**: Snowflake requires such an upstream to expose a system-derived unique key or a
frozen region before an incremental/adaptive downstream may consume it — otherwise the rebuild
is an undiffable total change
([Snowflake](https://docs.snowflake.com/en/user-guide/dynamic-tables-performance-incremental-operators)).
**Fit**: smelt's graph layer marks a full-refreshed upstream as a total delta today (safe,
over-running). Snowflake's rule shows the refinement: with row identity (or C1's diff-then-
patch on the upstream), a full rebuild can still emit a *bounded* downstream delta. Ties B3,
C1, and C5 together at the graph layer.

---

## E. Patterns surveyed and deliberately not adopted (with reasons on record)

- **Snapshot-diff SCD2 with execution-time stamping** (SQLMesh `SCD_TYPE_2_BY_COLUMN` stamps
  `valid_from` with execution_time; `invalidate_hard_deletes` closes intervals at execution
  time on absence) ([SQLMesh model kinds](https://sqlmesh.readthedocs.io/en/stable/concepts/models/model_kinds/)).
  This is exactly the run-clock-stamped history smelt's §Limitations "No SCD2 over mutable
  snapshots" rejects: the history depends on when the build ran. SQLMesh is transparent about
  the consequence — the kind is non-idempotent and partial restatement is unsupported. The
  delete-by-absence *interval close* is separately interesting for the succession sketch's
  delete handling, but only when the absence signal is itself an event-timed fact (a CDC
  delete), which the sketch already covers.
- **Declared non-idempotent keyed upsert** (SQLMesh `INCREMENTAL_BY_UNIQUE_KEY`: explicitly
  non-idempotent, restatement requires full rebuild). smelt's equivalence invariant is
  precisely the refusal of this posture; recorded here as the field's honest label for what
  smelt's key grain proves instead.
- **DRed over-delete-then-rederive** ([SIGMOD '93](https://dl.acm.org/doi/10.1145/170035.170066))
  **[verified]** — the deletion algorithm for *recursive* views (delete a superset, rederive
  survivors). smelt has no recursive models; irrelevant until recursive CTE models are ever
  admitted, at which point it is the standard answer. Its sibling contribution, **view
  adaptation** (incrementally adapting a view when the *definition* changes), is subsumed for
  smelt's purposes by C1's diff-then-patch route.
- **Shared arrangements / cross-view state sharing and late materialization**
  (Materialize: one maintained index shared by all views needing it; key-only state with
  payload fetch on demand) ([arrangements](https://materialize.com/docs/get-started/arrangements/),
  [delta joins](https://materialize.com/blog/delta-joins/)). The push-based multiversioned-index
  mechanism doesn't transfer to batch; the transferable shadow is "shared materialized helper
  state across models", which is A4's view-tree territory and already smelt's cross-model
  planner ambition. Delta-joins (per-input delta paths over shared indexes, zero intermediate
  state) likewise assume resident indexes; the batch analogue is just the delta-probe smelt's
  mutation cells already do.
- **DBSP as the unifying theory** ([VLDB J. '25](https://link.springer.com/article/10.1007/s00778-025-00922-y)) —
  incrementalize any query by composing per-operator incremental primitives, subsuming the
  historically separate query classes. smelt's posture (per-cell admission over a typed
  registry, engine delegation beyond the ladder) is deliberately narrower: smelt emits SQL
  against stock engines rather than operating a Z-set runtime. DBSP remains the reference for
  *why* the ladder's rungs are what they are, and Feldera-class engines are
  `refresh: materialized_view` delegation targets, not a smelt-emitted pattern.

## Summary ranking (by fit ÷ machinery)

1. **A1 per-group recompute escape hatch** — converts the non-invertible-retraction refusal
   into a bounded repair with existing machinery.
2. **C1 diff-then-patch reconciliation write** — one new registry entry serving recompute
   cells, CoW cost, CDC-clean deltas, and the changed-column definition trigger.
3. **B3 i-diff / FD-targeted UPDATE** — the literature anchor for an already-contemplated
   registry entry; adopt its admission vocabulary.
4. **A3 ordered-semigroup rung** — names a ladder rung smelt half-implements, admitting
   latest-by-event-time columns as derived verdicts.
5. **A2 derivation counts** — unlocks DISTINCT/EXISTS under retraction at O(|output|) state.
6. **B1 outer-join clean-up delta**, **A5 ranked-prefix**, — next classifier families after
   succession, sharing its walk-vocabulary investment.
7. **D2 metadata-operation trigger**, **C2 shadow swap**, **C4 storage-merge lowering** —
   registry/trigger growth gated on backend capability work.
8. **B2 self-maintainability axis**, **D1 freshness barrier**, **A4 view trees** — real but
   architectural; only with demonstrated demand.

## References

Primary sources cited inline. Sibling documents: `docs/specs/incremental_models.md`
(the registry this measures against), `docs/research/20260723-scd2-succession-pattern.md`
(the recognition-over-declaration precedent), `docs/research/20260703-model-updates.md`
Part 12 (rejection-catalogue validation; this note is its mechanism-side complement).
