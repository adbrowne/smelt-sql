---
feature: model_transforms
status: experimental
last_reviewed: 2026-07-11
owners: [andrew]
---

# Model Transforms

> **What this is.** The catalogue of **physical execution mechanisms** a model's
> properties license — general, reusable transforms usable well beyond refresh
> maintenance (backfills, schema evolution, general execution). Each transform
> names the property or world-fact that licenses it, the mechanism it emits, and
> the invariant it preserves. It defines *mechanisms*, not *modes*: it does not
> decide when a mode selects a transform (that composition is `incremental_models.md`),
> nor prove the properties that license one (`model_properties.md`), nor own the
> **processed-input equivalence invariant** the transforms serve (defined once in
> `incremental_models.md` §"The equivalence invariant" — referenced here, never
> redefined). Out of scope, with their own homes: mode-only transforms that are
> meaningless outside a single `refresh:` mode (the shape-profile
> sections of `incremental_models.md`); the backend capability flags a transform's lowering
> checks (`multi_backend.md`); the `refresh:` enum and the three-state declaration
> law (`models.md`).
>
> **Spec-first rule.** Edit this file before writing the implementation plan. The spec diff is the change description.
>
> **Timeless-oracle rule.** This spec describes the transforms as if they have always existed. Implementation status lives in §Known Divergences (behaviour + plan link) or §References → Plans (history).

## Surface

The callers of this spec are the refresh-mode specs, the planner, and the
runtime emitter. The surface is the transform catalogue: each transform is a
named physical mechanism, licensed by exactly the property or world-fact in its
row, that a mode composes by name.

**Placement criterion.** A mechanism is catalogued here **iff it is
meaningful/stateable without naming a single refresh mode** — a general
capability a property licenses. A mechanism meaningful only inside one mode
stays in that mode's spec (see §Semantics → *Transforms that stay in a mode spec*).

| Transform | Licensed by (property / world-fact) | Mechanism | Maturity |
|---|---|---|---|
| Keyed `merge_into` (target-as-replica) | inverse-free / monoid rung | fold a keyed delta into stored state; matched keys update, unmatched insert; never re-read history | **built** |
| Windowed-keyed-maintenance driver | driving-fact / anchor + monoid rung | sequence `merge_into` across driving partitions: classify → step over partitions in temporal order → per-partition pushdown → create-or-merge | **built** |
| Source-filter pushdown (window-an-input) | monotonicity trace + derived bound | wrap each bounded input ref in a `partition_column` subquery so the scan reads only its window | **built** |
| Partition DELETE+INSERT | trace + partition alignment | delete the touched half-open partition range `[start, end)`, then insert the rebuilt rows | **built** |
| Outer output-clamp | event-time projection (needs no proof) | wrap the model in a projection over its output schema (`SELECT * FROM (<model>) AS _smelt_output_clamp WHERE <col> …`), filtering rows to the write window on the projected `event_time` | **built** |
| Generic column-scoped merge (targeted write) | bounded footprint + well-defined mutation-sensitivity group | `MERGE`/`UPDATE ... FROM` restricted to one mutation-sensitivity column-group's columns, keyed where the source is keyed; the dimension-driven horizon MERGE and the upstream-re-deriving half of field-backfill are named instances | **built** |
| Change-suppressed MERGE (keyed `merge_into` / column-scoped merge variant) | region row identity + change comparability on every compared column | matched-arm gains `AND (t.c1 IS DISTINCT FROM s.c1 OR …)` over the cell's comparable mutation-sensitive columns, so a row whose applied effect is the identity is never written; the unmatched side is dialect-keyed (`WHEN NOT MATCHED BY SOURCE` where the dialect has it, else a separate scoped `DELETE` in the same statement group) | **built** (column-scoped and keyed-fold) |
| Staged-candidate conditional DELETE+INSERT (merge-less realisation) | region row identity + change comparability on every compared column | stage the candidate region once into a temp relation, derive changed/new/departed row sets by diff joins (keyed identity) or `EXCEPT ALL` both ways (whole-row identity), then `DELETE` the changed-or-departed rows and `INSERT` the changed-or-new rows in one transaction — the keyed-shaped conditional write for backends without `MERGE` | *partial* (keyed identity only) |
| Two-layer widened-scan + exact output clamp | finite frame reach `k` | scan `[out_start − k − offset, out_end)`, clamp output to the derived output window `[out_start, out_end)`: read the margin, never re-write it | **built** |
| Output-window derivation (partition-column skew inversion) | derived partition-column skew bound (Form B relation between the driving date column and a derived `partition_column`) | invert the declared relation to map the run window `[start, end)` to the output window `[start − after, end + before)`; identity (no skew) yields `output window = run window` | **built** |
| UNION-branch wrap-and-filter | set-operation distribution + per-branch trace | inject the source filter independently into each `UNION`/`INTERSECT`/`EXCEPT` branch | unbuilt |
| Hidden decomposed state + presentation view | decomposed-monoid rung | store the monoid element (`(sum,count)` / Welford / HLL), expose the user value through a pure presentation view `π(state)` | **built** |
| Retraction via delta history | group (invertible) rung | store the invertible per-partition delta; on reprocessing subtract the old contribution, then add the new | unbuilt |
| Explicit bounded-domain multiset state | bounded-domain budget assertion | store a per-key value→count multiset (a bounded-domain Z-set); one state serves many presentations and free retraction | unbuilt |
| Compile-time pinning | run-determinism (`NOW`/`CURRENT_*`) | resolve a run-deterministic function to a single literal once per run, before emit | **built** |
| Definition-change field-backfill: in-place `UPDATE` | additive-only model diff, payload field is a pure function of stored columns | backfill the added column with an in-place `UPDATE`, no upstream re-read; refused (`MaintenanceSkeletonColumnAdded`) if the field lands in a skeleton (identity/grouping/dedup/ordering) position — that is a grain change, not a backfill | **built** |
| Definition-change field-backfill: keyed column-scoped `MERGE` | additive-only model diff, payload field re-derives from upstream | backfill the added column by re-deriving from upstream via the generic column-scoped merge, keyed where the source is keyed, inheriting that source's partition-locality verdict unchanged; refused (`MaintenanceSkeletonColumnAdded`) in a skeleton position for the same reason | unbuilt |
| Dimension-driven horizon-bounded MERGE | target-as-replica + join-contribution monotonicity + a **derived** horizon `H` | merge a dimension batch straight into the target slice `[conv_ts − H, conv_ts]`; never re-read the fact. Licensed by a *derived* `H` only — a *declared*-on-source `H` no longer licenses this transform (an under-declared source lateness would silently truncate the recompute); where `H` is not derivable the transform is simply not licensed and the enrichment evaluates via the ordinary widened scan | **built** |
| Horizon settled-delay / tail-rewrite | maintained-window / **derived** horizon derivation | for a forward-reach (late-arriving) source, hold the write until the derived horizon has settled, or rewrite the tail slice within the horizon on a later run; the write clamp tracks the *derived* horizon, never a declared value. Batched-side forward-reach machinery, confirmed derived-only (never licensed by a declared horizon) | unbuilt |
| Reconciliation-ledger fold | additive column-group algebra | consult the `(output-region × column-group)` ledger entry before merging: refuse (never fold) a delta already in its processed set, otherwise combine and extend it; required by any non-idempotent (additive-fold) combiner, which must refuse a re-run of a ledgered window exactly, not best-effort | unbuilt |
| Reconciliation-ledger recompute-reset | a region recompute | reset every ledger entry the recompute's footprint intersects to exactly the input the recompute read, so a later fold cannot double-count against stale bookkeeping | unbuilt |
| Idempotent window re-scan vs delta-driven probe | idempotent monoid + source mutation profile | unconditional CDF-free re-scan when the fold is idempotent; a per-run changed-set probe when a change feed is available | *partial* |
| Delta-restricted enrichment join | skeleton-source closure (`model_properties.md`) + an exact upstream delta on the driving-side model edge | restrict the enrichment recompute's driving scan to the delta's key set via a semi-join, replacing the widened scan for that cell; restricts recompute *breadth* under an exact delta, never what is scanned into `S` — composes with, but is licensed independently of, write suppression | *partial* (maintained-model edges only) |
| Delegate-to-native-IVM | `supports_native_ivm` + engine gate | emit the backend's own maintained object; hard error if the engine rejects the query | *partial* |
| DAG composition | litmus rule (`models.md`) | express a mode combination as two composed models at two grains, not a new mode | mechanism exists |
| Full refresh | — (universal fallback) | drop and rebuild the whole output; the honest verdict for an unmaintainable declared mode | **built** |
| Backend lowering / emulation | backend capability flags | lower a logical primitive to the engine (native `INSERT OVERWRITE` / create-or-replace, or DELETE+INSERT emulation; cross-engine `read_parquet`) | **built** |

## Semantics

Every transform preserves the **processed-input equivalence invariant**
(`incremental_models.md`): the physical result equals what a full refresh over the
same processed inputs would produce. A transform that **cannot** preserve it for a
given model is **refused with a diagnostic, never applied approximately** (see
§Constraints). The load-bearing mechanics:

**Keyed `merge_into` (target-as-replica).** The stored table *is* the keyed state
(one row per key). A run computes the delta over new inputs and folds it into the
target — matched keys update, unmatched insert — without re-reading history. Sound
only on the monoid rungs of the ladder (`incremental_models.md`); an invertible
combiner is required to *un-see* a contribution under reprocessing (handled by
retraction-via-delta-history, not by `merge_into` alone). The step loop that
sequences `merge_into` across driving partitions (classify → step → per-partition
pushdown → create-or-merge) is the *windowed-keyed-maintenance driver* — a mode-agnostic
mechanism, so it is catalogued here in its own right. Its reference *implementation*
lives today in the key-grain profile's built seed (`incremental_models.md`
§"The key grain"; the direct-monoid classifier, the only
keyed mode built so far) and generalises across `keyed`'s own column families as they
land, with `versioned` a prospective future consumer of the same driver; the normative
*description* of the driver mechanism is this catalogue entry, not the mode spec.

**Source-filter pushdown + the two clamps.** Three related mechanisms share one
window. Source-filter pushdown wraps each bounded input ref in a subquery so the
scan is pruned at the source. The outer output-clamp filters the outermost
projection so only the write window is emitted; it is applied to a **wrapping
projection over the model's output schema** (`SELECT * FROM (<model>) AS
_smelt_output_clamp WHERE <col> …`), never spliced into the model's own
outermost `WHERE` — the clamp ranges over the output schema by output column
name, so it binds unambiguously even when several FROM items expose the same
column name (a self-referential model, two same-named timeseries sources), and
it filters output *rows*, evaluated after any window function the outermost
`SELECT` computes. The clamp column is an **unqualified** column of the model's
output schema; a qualified (dotted) name is rejected — an inner-alias
qualifier is definitionally out of scope in the wrapping projection.

**The output window is derived, never assumed.** The window the two clamps
share — the **output window**, the partition range this run writes — is a
function of the run window and the model's declared time relations, not the
run window verbatim:

- **Identity (the common case).** When the `partition_column` tracks the
  event time driving new data (the same column, or a pure truncation of it),
  new rows land in the partitions of their own window: `output window = run
  window`.
- **Skew inversion (derived `partition_column`).** When the `partition_column`
  is *derived* and can skew away from the driving date column — declared by a
  Form B relation in the model's SQL, `driving_date BETWEEN partition_column −
  before AND partition_column + after` — new data in `[start, end)` can change
  partitions outside the run window. The output window is the relation's
  **inversion**: `[start − after, end + before)`. A session model partitioned
  by `session_start_date` under a 1-day cap (`before = after = 1 day`) run for
  `[D, D+1)` therefore has output window `[D−1, D+2)`: an event on day `D`
  extending a session rooted on `D−1` rewrites the `D−1` partition in the
  same run. A side of the inversion the data can never realise (here the
  leading day: a session cannot start after its own events) simply recomputes
  to an unchanged partition — the derivation stays purely declarative.

  The declared relation is also a **semantic cap**, not a heuristic: a row of
  the driving date column that falls outside the declared relation of a
  partition never contributes to that partition, in *any* build shape. An
  entity that would naturally chain past the declared bound — a session whose
  events span two midnights under a ±1-day declaration — is **truncated at
  the declared bound**, and identically so in an incremental run and a full
  rebuild, because the relation is part of the model's own SQL. Truncation is
  therefore never an incremental artifact and never a processed-input
  equivalence violation; a model that must not truncate widens its declared
  relation, which widens the derived output window with it.

When the model has a **finite frame reach `k`** (a `RANGE … INTERVAL` window,
an interval join), the two-layer widened-scan reads a margin **relative to the
derived output window** — `[out_start − k − offset, out_end + k′)` — wide
enough to recompute *every written partition* correctly at its own edges,
while the output clamp restricts writes to exactly `[out_start, out_end)`:
the margin is *read but never re-written*, which is what keeps the result
partition-equivalent. Sizing the scan from the run window instead of the
derived output window would rewrite a skew-reached neighbour partition from
a scan too narrow for *its* reach — the corruption the exact-clamp split
exists to prevent. For the transparent single-source, zero-margin,
zero-skew case the pushdown filter *is* the clamp (same window by
construction) and the outer clamp is dropped as textually redundant.
UNION-branch wrap-and-filter is the same pushdown distributed independently
over each set-operation branch.

**Hidden decomposed state + presentation view.** The stored column is a monoid
element that is not itself the user value; the user value is a pure function
`π(state)` exposed through a presentation view. `merge_into` maintains the state
element; `π` never touches history. Sound iff `π` is a pure function of one
consistent state row.

**Retraction via delta history** and **explicit bounded-domain multiset** are the
two ways to regain fidelity the plain monoid loses: the first stores invertible
per-partition deltas so a contribution can be subtracted before re-adding (the
group rung, for corrections/deletes); the second stores a per-key value→count
multiset so exact holistic aggregates (`MEDIAN`, exact `COUNT(DISTINCT)`) are
maintainable. The multiset is **opt-in and fail-loud**: state is `O(active
domain)`, so it is applied only under a declared bounded-domain budget and the
runtime caps it with a full-refresh fallback.

**Generic column-scoped merge** is the general targeted-write primitive: a
`MERGE`/`UPDATE ... FROM` restricted to the columns of one mutation-sensitivity
group, keyed where the source is keyed, licensed by a bounded write footprint
(the reflection of the scan bound onto a bounded set of output addresses) and a
well-defined mutation-sensitivity partition. **Dimension-driven horizon MERGE**
and the upstream-re-deriving half of **definition-change field-backfill** are
named instances of it, each adding its own licensing property on top: the
horizon MERGE additionally needs target-as-replica plus a monotone join
contribution plus a **derived** bounded horizon `H`, merging a dimension batch
directly into the target slice `[conv_ts − H, conv_ts]` without re-reading the
fact.

**Change-suppressed MERGE and the staged-candidate conditional DELETE+INSERT** are *variants* of
the write transforms above — a licensing property admits each variant into a cell's plan space,
it never chooses between a variant and its unconditional sibling (§Design "A property licenses;
it never chooses"). Both realise the same no-op write elimination
(`incremental_models.md` §"Windowed maintenance and the horizon" category 2): a maintenance write
is skipped exactly where the row's applied effect is proven, per row by evaluation, to be the
identity. Two obligations license either variant, both discharged fail-closed:

- **Region row identity** — the rows the compare joins stored state to candidate rows on: a
  declared `unique_key`, else a proven grain key, else whole-row multiset identity (`EXCEPT ALL`
  both ways) where no key is available. A cell whose identity cannot be established this way is
  never conditionally written.
- **Change comparability on every compared column** — each column in the predicate must be a pure
  function of the processed inputs, so re-evaluating it at a fixed processed-input set reproduces
  the same bits. A column that legitimately varies run to run (a declared `contract: plausible`,
  or a run-pinned `NOW()`) is incomparable; a cell whose compared column-group contains even one
  incomparable column refuses the conditional variant entirely and keeps the unconditional one —
  comparing only the mutation-sensitive group is sound because the other groups are proven
  insensitive to the trigger.

Both variants carry the same **fixed-`S` bit-equality obligation**: at a fixed processed-input set
`S`, the conditional variant and its unconditional sibling must produce identical stored state —
this is what makes them *interchangeable* techniques for a cell (§"The plan matrix"
"Interchangeability and choice") rather than a separate mode. Choosing between them is therefore a
cost-model/`prefer`/`technique` matter, never a correctness one; the variant only changes *whether*
an unchanged row is physically rewritten, never which bits a rewrite would produce.

**Change-suppressed MERGE** is the matched-arm suppression: the existing `merge_into`/generic
column-scoped merge emitter gains `AND (t.c1 IS DISTINCT FROM s.c1 OR …)` over the compared
column group on its matched arm, so an unmatched-effect row is skipped rather than rewritten. The
unmatched-by-source side (a row present in stored state but absent from the candidate set, for a
region-scoped variant) is dialect-keyed: `WHEN NOT MATCHED BY SOURCE` where the dialect exposes
it, else a separately emitted scoped `DELETE` inside the same statement group.

**The staged-candidate conditional DELETE+INSERT** is the merge-less realisation of the same
licence — the keyed-shaped conditional write for a backend that cannot run `MERGE` at all (a
documented gap: Spark-over-Parquet). One transaction: stage the candidate region into a temp
relation; derive the changed/new/departed row sets against stored state by diff joins (keyed
identity) or `EXCEPT ALL` both ways (whole-row identity); `DELETE` the changed-or-departed rows;
`INSERT` the changed-or-new rows. Byte-equivalent to today's region DELETE+INSERT at fixed `S`,
with the write physically restricted to the rows whose effect is not the identity.

**Definition-change field-backfill** is the pair of techniques a model gaining
output fields backfills with (`incremental_models.md` §"The definition-change
trigger"), chosen by what the added field reads: a payload field that is a
*pure function of stored columns* backfills as an **in-place `UPDATE`** (no
upstream read), admitted only under the additive-only model-diff proof; a
payload field that *re-derives from upstream* backfills as the **generic
column-scoped merge**, keyed where the source is keyed, inheriting that
source's partition-locality verdict unchanged. Both members of the pair are
**refused** with `MaintenanceSkeletonColumnAdded` — never applied — when the
added field lands in a **skeleton** position (identity/grouping/dedup/ordering):
that is a grain change, not a backfill, and the honest plan is a recompute
(effectively a new model). Fields added together factor by shared
mutation-sensitivity into one backfill op per group; a newly-added group's
backfill is always full-input, since there is no prior state of that column to
fold onto.

**The reconciliation-ledger fold/recompute-reset pair** is the bookkeeping every
additive column-group technique consults before it merges, per
`incremental_models.md` §"The reconciliation ledger": each `(output-region ×
column-group)` ledger entry records the processed-input vector the region has
already folded. **Fold** refuses (never merges) a delta already reflected in
the entry's processed set, otherwise combines and extends it — the real
obligation behind "never fold a delta twice," required by any non-idempotent
(additive-fold) combiner. **Recompute-reset** is the other side: a region
recompute resets every ledger entry its footprint intersects to exactly the
input it read, so a subsequent fold cannot double-count against stale
bookkeeping. The interchangeability rule that licenses choosing between two
techniques for one cell holds only modulo this ledger: fold-then-recompute is
safe (the recompute resets the region's ledger), but recompute-then-refold
double-counts.

**Compile-time pinning** resolves a run-deterministic function (`NOW()`,
`CURRENT_DATE`) to one literal per run so every partition of that run sees the same
value — the equivalence invariant would otherwise be violated by a value that
drifts across a chunked backfill. A genuinely non-deterministic function
(`RANDOM`, `UUID`) is *not* pinnable and is refused unless the column is declared
exempt.

**Delegate-to-native-IVM, full refresh, backend lowering.** Delegation emits the
engine's own maintained view and hard-errors when `supports_native_ivm` is false —
it runs no smelt combiner and is where smelt-driven maintenance ends. Full refresh
is the universal fallback that always upholds the invariant trivially. Backend
lowering/emulation maps a logical primitive to whatever the engine supports
(native `INSERT OVERWRITE`, or DELETE+INSERT emulation; cross-engine transfer via
Parquet), gated on the backend capability flags in `multi_backend.md`.

### Transforms that stay in a mode spec

These are meaningful only *inside* one refresh mode and are **not** catalogued
here; the mode spec owns them in full:

- **Backfill chunking** (one-shot / auto-sized / per-partition) and **auto-coarsen
  run window** — `incremental_models.md` §"First-run and backfill".
- **Close-old / open-new interval maintenance** (SCD2) — `incremental_models.md`
  §"Close-old / open-new interval maintenance (the combiner)".

**Deferred, not catalogued as built or unbuilt: eviction / settled-key GC.** Retiring
keyed state older than `current_window − H` is **not** licensed by any transform today —
`incremental_models.md` §"No write-eligibility clamp" removed the write-eligibility clamp that
would have motivated it (`docs/research/20260705-keyed-collapse-application.md` D6). It
is deliberately deferred rather than catalogued as a mode-local transform: if it is ever
introduced it must ship together with late-fact accounting (a package, not a standalone
GC pass), tracked as a deferred item rather than given a home in any single mode spec.

## Design

**Named for the mechanism, not the mode.** A keyed `merge_into`, a source-filter
pushdown, a targeted backfill are each useful for backfills, schema evolution, and
general execution — not only refresh maintenance — so they are catalogued as
capabilities and composed by name, rather than re-described inside each mode's
spec. This is what gives `smelt:validate` one home per mechanism to check for
drift.

**The placement criterion is definitional, not a consumer count.** A transform
lives here iff its mechanism is stateable without naming a mode. This is why
pushdown, DELETE+INSERT and the clamps live here even though only `batched` drives
them today, while backfill chunking — which has no meaning outside batched
execution — stays in `incremental_models.md`'s partition-grain profile. Building a transform before a second
consumer exists is fine (they are broadly useful); the criterion governs *where the
text lives*, never *when it is built*.

**A property licenses; it never chooses.** Each row names exactly the property or
world-fact that makes the transform sound. The transform is applied only when that
licence holds and is otherwise refused — the machinery is a validator, never a
chooser (`incremental_models.md` §"Validator, not chooser"). This keeps the mapping
property → transform auditable and forbids an approximate application when the
licence is absent.

**The ladder is the maintainable/delegated boundary.** `merge_into`,
decomposed-state-plus-view, retraction, and the multiset are the mechanisms of
rungs 1–4 of the algebraic ladder; delegate-to-native-IVM is what lies beyond it.
The ladder itself (its ordering and cutoff) is owned by `incremental_models.md`;
this spec only realises each rung as a physical transform.

**Rejected: auto-widening the write window to the scan margin.** An earlier
runtime widened the *written* partition range to cover a window's lookback
rather than only widening the *scan*. That double-counts at partition edges and
was redesigned into the two-layer widened-scan/exact-clamp split (read the
margin, write only the window). The write window must equal the output window;
only the scan may be wider. This rejection is **not** a rejection of the
output-window derivation (§Semantics): a skew-inverted output window is not a
widened write — it is the *correct* output window, with each written
partition's scan sized from that window's own reach. The two are distinguished
by what sizes what: margin-widening let the *scan bound* leak into the write
range (wrong direction); derivation computes the write range from the declared
relation and then sizes the scan from it (right direction).

**Derived output window composes with chunking; it never forces one wide
write.** Controlling per-query write size is a first-class production concern:
a job with a multi-day skew or lookback is routinely run as several sequential
bounded updates rather than one large one. The derived output window is a
*range to be covered*, not a mandate for a single statement — backfill
chunking (`incremental_models.md` §"First-run and backfill") splits it into
sequential DELETE+INSERT pairs exactly as it splits a wide run window, each
chunk's scan sized from that chunk's own reach. *Scheduling a separate re-run
of the earlier calendar window* was rejected as the primitive: a calendar-window
re-run is a whole-DAG event (and is refused outright by non-idempotent keyed
models' reconciliation ledger), whereas the skew is a per-model fact — deriving
the output window applies it exactly where it holds and nowhere else.

## Constraints & Invariants

- **Equivalence or refusal.** A transform is applied only when its licensing
  property is proven or declared. A transform that cannot preserve the
  processed-input equivalence invariant (`incremental_models.md`) for a given model
  is **refused with a diagnostic** — never applied approximately, and never with a
  silent fallback to a default.
- **Write window = output window; scan window ⊇ output window.** Any widened-scan
  transform may read a margin but must clamp writes to exactly the output window.
  The output window is **derived** from the run window via the model's declared
  partition-column relation (identity when the partition column tracks event
  time; skew-inverted under a Form B relation on a derived partition column —
  §Semantics "The output window is derived, never assumed"), and every written
  partition's scan is sized from the derived output window's reach, never from
  the run window's.
- **`merge_into` requires a monoid rung; reprocessing requires invertibility.**
  A non-invertible combiner under reprocessing is refused (or routed to full
  refresh), never merged approximately.
- **The bounded-domain multiset is opt-in and capped.** It is applied only under a
  declared budget and fails loud (full-refresh fallback) when the domain exceeds
  the cap; it is never the default for a holistic aggregate.
- **Native-IVM delegation is fail-loud.** Delegate-to-native-IVM hard-errors when
  the backend does not support it; it never silently downgrades to a smelt-driven
  approximation.
- **One home per mechanism.** A transform meaningful only inside a mode is not
  catalogued here; the mode spec owns it (see §Semantics).

## Known Divergences / Open Questions

The whole catalogue is being consolidated and largely unbuilt; status is tracked
by `docs/plans/20260704-model-updates.md` (design:
`docs/research/20260704-maintenance-fundamentals.md`).

- **Built today:** keyed `merge_into` (the `Backend::merge_into` trait method,
  impls in `smelt-backend-duckdb`/`-spark`); source-filter pushdown
  (`inject_source_filters`); partition DELETE+INSERT (`delete_partitions` +
  `insert_into_from_query`); outer output-clamp (`inject_time_filter`);
  output-window derivation (`smelt_logical::analysis::walk::model_partition_skew`
  derives the model's own partition-column skew bound, consumed by
  `crates/smelt-runtime/src/windowing.rs::compute_incremental_windows` to
  widen the run window into the output window before chunking) — the
  two-layer widened-scan + exact output clamp split (the scan reads
  `[out_start − k − offset, out_end)`, and the output clamp and the DELETE
  partition range both use the derived output window `[out_start, out_end)`,
  which equals `[start, end)` for an identity `partition_column` and the
  skew-inverted range otherwise); the transparent single-source, zero-margin,
  zero-skew fast path (`is_transparent_single_source` composed with a
  `Skew::ZERO` check at its call site), which skips the outer clamp entirely
  since the source-level filter already is the output clamp; full refresh;
  backend lowering/emulation (`insert_overwrite`,
  cross-engine Parquet); the in-place-`UPDATE` half of definition-change
  field-backfill (`crates/smelt-runtime/src/backfill.rs::targeted_column_backfill`),
  which builds the `UPDATE ... FROM (...) AS src` statement licensed by an
  additive-only model diff and a non-empty `unique_key` (the keyed
  column-scoped-`MERGE` half, and the `MaintenanceSkeletonColumnAdded` refusal,
  are unbuilt); dimension-driven horizon-bounded MERGE
  (`crates/smelt-runtime/src/dimension_horizon_merge.rs::dimension_horizon_merge`),
  which clamps a dimension batch's recompute `SELECT` to `[conv_ts − H,
  conv_ts]`, licensed by a monotone join contribution
  (`join_contribution_monotone`) and a bounded horizon `H` (the forward
  `after` reach from `derive_model_bounds`) — and is now actually handed to
  `Backend::merge_into` by a caller: `crates/smelt-runtime/src/
  maintenance_driver.rs::execute_column_scoped_merge` is the physical
  executor for a `Technique::ColumnScopedMerge` plan cell whose scan
  locality is a genuine derived clamp (`PartitionLocal::Yes`), reached
  through `maintenance_driver::decide_column_merge_dispatch` in the regular
  incremental run loop (`smelt-runtime::execute_project`) — the SAME
  automatic, per-run dispatch path the accepted-full-scan corner
  (`PartitionLocal::No`, `execute_column_scoped_merge_full`) already uses in
  production. A plan cell the derivation did not admit, a backend that does
  not advertise `Backend::supports_column_scoped_merge`, or an unproven
  join contribution never reaches either executor. `incremental_models.md`
  §Known Divergences has the caller-side detail, including which corner is
  reachable from a real workspace fixture today.
- ~~The output clamp is injected at the model's own SELECT level.~~ Resolved:
  the clamp is applied to a wrapping projection over the model's output schema
  (§"Source-filter pushdown + the two clamps"), which closes both defects of
  the same-level injection — the binder ambiguity when several FROM items
  exposed the clamp column's name (a self-referential model, two same-named
  timeseries sources), and the window-function hazard where a same-level
  `WHERE` filtered the rows *feeding* a bare outermost window function,
  undercutting its widened-scan margin.
- **Skew-anchor matching is name-only; a table-qualified anchor on a foreign
  table can false-positive.** The partition-skew classifier
  (`smelt_logical::analysis::source_bounds::derive_partition_skew`) matches a
  Form B anchor by identifier name, accepting a table-qualified form — so a
  relation like `a.d2 BETWEEN b.d - INTERVAL '1 day' AND b.d + INTERVAL '1
  day'` anchored on an *upstream table's* column `b.d` matches a model whose
  own `partition_column` happens to be named `d`, even when that column is a
  straight passthrough with no derivation. The consequence is an over-wide
  (never under-wide) derived output window: neighbour partitions are
  DELETE+INSERTed unnecessarily and recompute to themselves — wasteful, never
  incorrect (the inversion only ever widens the write, and each written
  partition's scan is sized for its own reach). Avoidable by naming the
  model's output column distinctly from the join anchor. A precise fix would
  require the anchor to be provably the model's *own output column*, not any
  same-named qualified column. Tracked in
  `docs/plans/20260711-derived-output-window.md`.
- **Ordered (convergent self-edge) execution skips output-window derivation.**
  A self-referential model proven to converge partition-by-partition
  (`incremental_models.md` §"Window independence and self-referential models")
  builds strictly sequential single-partition batches over the run window
  verbatim — its self-edge's own bounding relation (e.g. `bal.d >= t.d −
  INTERVAL '1 day'`) is the windowed-driver mechanism's convergence bound,
  not a partition-column skew declaration, and (per the name-only matching
  above) would otherwise read as a spurious skew whenever the self-referenced
  column shares the model's partition-column name. Whether a genuinely
  skewing derived `partition_column` can compose with ordered self-referential
  execution at all is undecided; today the combination simply does not widen.
  Tracked in `docs/plans/20260711-derived-output-window.md`.
- **Change-suppressed MERGE is built for the column-scoped and keyed-fold families; the region
  DELETE+INSERT family stays unconditional.** The column-scoped `MERGE` emitter
  (`smelt_logical::maintenance::emit::emit_column_scoped_merge_suppressed`) and the keyed-fold
  `MERGE` emitter (`emit_keyed_fold_suppressed`) both gain a matched-arm `IS DISTINCT FROM`
  predicate over the cell's comparable columns — the keyed-fold variant compares the stored
  value against the fold's own combine expression, since a keyed fold's matched arm never copies
  a delta column verbatim — admitted fail-closed by `maintenance::choice::resolve_write_suppression`
  over the region row identity (P2) and per-column change comparability (P3) proofs. The region
  DELETE+INSERT family still rewrites its whole window unconditionally.
- **The staged-candidate conditional DELETE+INSERT is built for the keyed-identity realisation
  only; the whole-row `EXCEPT ALL` realisation remains unbuilt.** `smelt_logical::maintenance::
  emit::emit_staged_candidate_conditional` emits the merge-less keyed-shaped write — stage the
  candidate region into a temp relation, `DELETE` the rows a declared/proven key identifies as
  changed, `INSERT` the changed-or-new rows read back from the staged relation, `DROP` the temp
  relation — as one transaction, so a mid-group failure leaves both the target and the temp-
  relation namespace untouched. `maintenance::choice::resolve_keyed_write_mechanism` chooses
  between the keyed `MERGE` and this mechanism from a backend-capability flag alone (never a
  silent substitution on a `MERGE`-capable backend); a `write:` pin over this choice, the
  whole-row (`EXCEPT ALL`-both-ways) realisation for a keyless region, and wiring this choice
  into the live `refresh: keyed` per-partition execution loop (`smelt-runtime::cumulative`) all
  remain open, tracked by `docs/plans/20260715-composed-axes-conditional-maintenance.md`.
- **Delta-restricted enrichment join is built for a maintained-model edge's own driving-source
  recompute.** `maintenance::derive::append_model_edge_cells` derives the skeleton-source-closure
  verdict (P1, `model_properties.md`) shared by every model edge of a downstream model;
  `maintenance::choice::resolve_recompute_restriction` admits the restriction only when that
  verdict is `Closed` *and* the driving edge's own recorded observed delta (T5) is present and
  non-empty; `maintenance::emit::emit_delete_insert_delta_restricted` emits the semi-joined
  `DELETE`+`INSERT`. The transform is per-cell: an absent closure proof, an `Open` verdict, or an
  absent/empty upstream delta all fall back to the ordinary widened scan
  (`maintenance::emit::emit_delete_insert`), byte-identical to the unrestricted statement — the
  restriction never changes what is scanned into `S`, only which rows the enrichment recompute
  re-derives. `smelt_runtime::maintenance_driver::execute_delete_insert_with_delta_restriction`
  reads the recorded delta and dispatches between the two forms against a real backend, and the
  runtime's own per-batch execution loop (`crates/smelt-runtime/src/execute.rs`) now calls it for
  every model-edge-sourced, `DeleteInsert`-strategy creation cell over an already-materialized
  target on a DuckDB run — the same dispatch decision (`resolve_live_delta_restriction_facts` +
  `build_delete_insert_group_dispatched`) also backs the `--dry-run`/`smelt explain` reporting
  branch, so the reported statement can never structurally diverge from what a live run with the
  same inputs executes. Extending the licence to an external `mutable_snapshot` source's own
  synthesized delta (the fingerprint sidecar, M3) remains unbuilt. Tracked by `docs/plans/
  20260715-composed-axes-conditional-maintenance.md`.
- **Unbuilt:** UNION-branch wrap-and-filter, retraction via delta history,
  bounded-domain multiset, compile-time pinning; the reconciliation-ledger
  fold/recompute-reset pair (no `(output-region × column-group)` ledger
  storage exists yet — every technique today behaves as if it always folds
  cleanly); the keyed column-scoped-`MERGE` half of definition-change
  field-backfill and its `MaintenanceSkeletonColumnAdded` refusal. Generic
  column-scoped merge has a standalone, admission-gated entry point today
  (`maintenance_driver::resolve_cell_technique`/`decide_column_merge_dispatch`
  + `maintenance_driver::execute_column_scoped_merge`, `incremental_models.md`
  §Known Divergences), but only one producer of the `dimension_batch_sql`
  it executes — the dimension-driven horizon MERGE's clamped `SELECT`; a
  second named instance (e.g. the keyed column-scoped-`MERGE` half of
  field-backfill above) would reuse the same executor but still needs its
  own `dimension_batch_sql` producer written. Idempotent re-scan vs delta
  probe is partial (input-delta discovery is partial).
- **Hidden decomposed state + presentation view is built as a mechanism**
  (`crates/smelt-logical/src/analysis/decomposed_state.rs`
  `decompose_to_state`): given a decomposable combiner (F4) it derives the
  hidden state columns and a presentation expression, refusing (never
  approximating) a holistic combiner, an unencoded state shape, or a
  presentation expression that fails the purity proof (F7). Only `AVG`'s
  `(sum, count)` state shape is encoded so far; wiring it as the driver
  (`merge_into`) for a live refresh mode (cumulative rung-2) is a later,
  mode-composition phase.
- **Duplicated licensing analyses.** Several transforms are licensed by proofs that
  exist in duplicate in the tree (two driving-fact resolvers); the licences these
  transforms read are being consolidated in `model_properties.md`. The interval-reach
  bound (`derive_model_bounds`) now has a single interval-literal parser and a single
  bound-derivation orchestration entry point (`derive_and_classify_bounds`), so a
  transform reads the derived bound from exactly one code path.
- **Horizon settled-delay / tail-rewrite is now catalogued (unbuilt).** Because the
  derived horizon is a core part of the maintenance contract (`incremental_models.md`
  §"Windowed maintenance and the horizon"), the forward-reach settle/tail-rewrite
  mechanism is catalogued above rather than deferred; only its implementation is
  outstanding, tracked by the same plan.
- The **windowed-keyed-maintenance driver** is a standalone mechanism (mode-agnostic
  classify → step over driving partitions in temporal order → per-partition pushdown →
  create-or-merge loop, fail-closed on a non-monoid combiner) with the built `cumulative`
  seed (now `incremental_models.md`'s key grain) as its first named consumer. `versioned` composes the
  same driver as it lands; the mechanism's normative home is this spec, not the mode
  that first implements it.

## References

- **Code**: `crates/smelt-backend/src/lib.rs` (`merge_into`, `delete_partitions`, `insert_into_from_query`, `insert_overwrite` trait methods); impls in `crates/smelt-backend-duckdb`, `crates/smelt-backend-spark`; `crates/smelt-runtime/src/transformer.rs` (`inject_source_filters`, `inject_time_filter`, `is_transparent_single_source`); `crates/smelt-runtime/src/compile.rs`.
- **Tests**: the batched per-partition full-refresh-equivalence harness; the cumulative end-state-equivalence harness; the pushdown/clamp unit tests in `smelt-runtime/src/transformer.rs`; the generative soundness oracle.
- **User docs**: the per-mode refresh pages under `docs-site/docs/`.
- **Plans (history)**: `docs/plans/20260704-model-updates.md`,
  `docs/plans/20260715-composed-axes-conditional-maintenance.md`.
- **Related specs**: `incremental_models.md`, `model_properties.md`, `models.md`, `materialized_view.md`, `multi_backend.md`, `timeseries.md`, `sources.md`, `schema_evolution.md`.
