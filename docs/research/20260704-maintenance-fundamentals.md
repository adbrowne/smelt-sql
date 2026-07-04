# Maintenance fundamentals: proofs, world-facts, and transforms as the spine of the refresh family

**Status:** research (decision-oriented; restructures how the maintained-model family is specced and planned)
**Date:** 2026-07-04
**Owners:** andrew
**Supersedes the *structure* of:** [`docs/plans/20260704-model-updates.md`](../plans/20260704-model-updates.md) — the mode-vertical master (Groups A/B/C/D). This note does **not** change the target behaviour that master implements; it re-cuts *how* it is decomposed so the commonalities land once.
**Related:**
- Research: [`docs/research/20260703-model-updates.md`](20260703-model-updates.md) — the exhaustive design this note re-indexes along a different axis.
- Research: [`docs/research/20260704-monotone-join-maintenance.md`](20260704-monotone-join-maintenance.md) — §9's "generalise across the refresh axis" is the seed of this note; its join/backfill fundamentals are folded in below.
- Specs to be re-homed: `models.md`, `batched_models.md`, `cumulative_aggregate.md`, `versioned_models.md`, `latest_value_models.md`, `materialized_view.md`, `accumulating_snapshot.md`, `timeseries.md`, `multi_backend.md`.

## Why this note exists

The maintained-model family is currently specced and planned **mode by mode**:
`batched`, `cumulative`, `latest_value`, `versioned`, `accumulating_snapshot`,
`materialized_view` each get their own spec and their own plan group. That
vertical cut hides the fact that the modes are built from a small, shared set of
underlying capabilities — and the hiding is not hypothetical:

- **The design docs already keep asking for the hoist** (paraphrasing below, not
  verbatim quotes). `20260703-model-updates.md` argues the monotonicity primitive is
  the shared blocker, to be built and tested *before any consumer* so the relaxations
  can't grow divergent private copies (§4.5); that partition alignment should be a
  scoped reusable signal that lands first as shared infrastructure (§9.5); that
  decomposability is one classifier with two consumers (§18.1); and that the analysis
  machinery is a **validator, never a chooser** (§17.6).
  `20260704-monotone-join-maintenance.md` §9 argues targeted backfill and the
  monotone-join MERGE belong at the `models.md` refresh-axis level, not as a per-mode
  capability.
- **The duplication is already in the tree.** An audit of `crates/smelt-logical`
  found six live copies of shared logic:
  1. Two independent interval-reach analyses — `analysis/temporal.rs`
     (`EffectiveWindow`, day-granular) vs `analysis/source_bounds.rs`
     (`BoundResult`, second-granular). Overlapping reach walks — the same
     INTERVAL/RANGE/LAG arithmetic — at different granularities (day vs second) and
     for different purposes (backfill classification vs pushdown pruning), with
     substantial duplicated logic rather than one shared walk.
  2. Three interval-literal parsers (`monotonicity.rs::parse_interval_value`,
     `source_bounds.rs::parse_interval_value_str`, `temporal.rs::extract_interval_days`)
     with subtly different unit handling (month = symbolic vs ≈30d).
  3. The `NONDETERMINISTIC_FUNCTIONS` list copied verbatim across
     `rules/incremental.rs` and `rules/cumulative.rs`, plus a third inline set in
     `monotonicity.rs`.
  4. Two unrelated driving-fact resolvers — `cumulative.rs` (ref-count-based) vs
     `source_bounds.rs::resolve_join_driving_fact` (alias-scoped trace).
  5. Two bound-derivation orchestration sites wrapping `derive_model_bounds`
     (`rules/incremental.rs` and `smelt-runtime/src/compile.rs`).
  6. Aggregate-name extraction done twice (typed `SqlFunction::is_aggregate` vs
     `cumulative.rs`'s string re-parse of the same select items).

Every mode-vertical phase that lands adds another private copy. The fix is to make
the shared spine a **first-class layer** — named, specced once, built once, and
*composed* by each mode.

## The thesis: one contract, three buckets, three declaration states

### The invariant everything serves

Above the buckets sits the parent correctness contract the whole family exists to
protect: **processed-input equivalence** (`20260703-model-updates.md` §1.2) — an
incremental run must produce the result a full refresh would, specialised to
*per-partition* equivalence for `batched` and *end-state* equivalence for the keyed
modes. Every proof below is a proof *in service of this invariant*; every transform
is licensed *because it preserves it*. The equivalence oracle (the generative
soundness net the master plan already runs as its regression net) is not one
fundamental among many — it is the thing the fundamentals establish. Layer 0 states
it first.

Two structural principles frame the invariant (`20260703` §1.3–1.4, §17.2):

- **Statefulness is the spine, not the selector.** Direct-state vs hidden-state
  (per-partition-rebuildable vs must-carry-accumulated-state) *explains* why the
  family splits into partitioned and keyed modes, but it must never become a user
  knob — the `refresh:` value is the selector.
- **Declare-as-selector vs declare-as-assertion.** The mode is a *selector* (it
  chooses execution). Every other declaration (source-lateness,
  `nondeterministic_columns`, a bounded-domain budget, a cost ceiling) is an
  *assertion* — it bounds or widens what the machinery may do, but never itself
  picks the strategy.

### The three buckets are stages of one pipeline

Everything the family does decomposes into three kinds of thing:

- **World-facts** — things smelt **cannot** derive and must be **told**, via
  frontmatter or a source declaration.
- **Proofs** — pure static analyses over a model's SQL (+ those declared facts)
  that return a *verdict*. What smelt establishes on its own.
- **Transforms** — physical execution mechanisms a proof (or a world-fact)
  *licenses*.

These are **not a clean partition of concepts — they are stages of a pipeline**:
`declare → prove → transform`. A single cross-cutting concern can thread through all
three. The clearest case is **input-consumption**, which `models.md` already models
as its own orthogonal axis: the *source mutation profile* is the world-fact,
*input-delta discovery* (window-forward / snapshot-diff / change-feed) is the proof,
and *re-scan vs delta-probe* is the transform — one pipeline, not three unrelated
rows. Layer 0 represents input-consumption as that cross-cutting axis (reconciled
with `models.md`, which today under-specifies it) and treats the buckets as pipeline
stages, so the same concept is not described three different ways.

A `refresh:` mode is then a **composition**: a named bundle of
`{consumed world-facts} × {required proofs} × {driving transform} × {output shape}`.

### The declaration law: three states, not two

`20260703` §17's "vertical is declared, horizontal is derived" is *almost* the law,
but a strict derive-vs-declare dichotomy mis-sorts two items, so state it as **three
states**:

- **Declared** — the `refresh:` mode (the one selector); plus the *assertions*
  (source mutation profile where not derivable, source-lateness,
  `nondeterministic_columns`, bounded-domain budget, cost ceiling, the
  declared-monotonicity escape hatch).
- **Derived** — the algebraic rung, the lookback/horizon, ordering, partition
  alignment, input-delta discovery, and monotonicity *where statically decidable*
  (the default — declared only as an escape hatch where undecidable).
- **Implied by the mode** — the output shape, and **who owns freshness** (pull for
  the smelt-driven modes, push for `materialized_view`): neither declared nor
  derived-from-SQL, but a consequence of the selected mode.

The analysis machinery *validates* the declared mode against the derived facts; **it
never chooses the mode.**

### The hoist rule (what keeps this tractable)

A fundamental is hoisted into the shared layer **iff ≥2 `refresh:` modes consume it**
— where "consumer" means a *mode* (`batched`, `cumulative`, `latest_value`,
`versioned`, `accumulating_snapshot`, `materialized_view`), **not** a sub-feature. A
batched sub-feature (join enrichment, a UNION branch) counts as consumer `batched`,
not as an extra consumer. Otherwise a fundamental stays `local:<mode>`. This is both
the anti-duplication rule *and* the anti-over-engineering rule. The inventory tags
every fundamental `shared` or `local:<mode>` on this strict mode-count basis.

**One deliberate exception, stated not smuggled:** a single-mode *mechanism* may be
built in the shared layer when a second mode consumer is *imminent and named*. Two
items invoke it — the **presentation-view** mechanism (only `cumulative` uses it
today, but `latest_value`/`accumulating_snapshot` hidden-state reuse is one phase
away) and the **windowed-keyed-maintenance driver** (only `cumulative` today; built
at the first of `latest_value`/`versioned`/`accumulating_snapshot`). Both are tagged
`shared*` in the inventory and governed by the ordering tie-breaker in the plan
section — they are **not** built before their second consumer exists.

**Scope of the rule.** The ≥2-mode test governs **proofs and transforms** (the shared
*analysis/execution* layer). **World-facts are declaration surfaces** and are not
consumer-counted: they live in the shared declaration layer (L3) by nature — declared
once on the source or frontmatter, consumed wherever — and are tagged `local:<mode>`
only when a *single* mode interprets them (e.g. `nondeterministic_columns` is
batched-only; a bounded-domain budget is cumulative-only).

## The inventory

Consumer keys: `ba`=batched, `cu`=cumulative, `lv`=latest_value, `ve`=versioned,
`as`=accumulating_snapshot, `mv`=materialized_view, `*`=all non-`full`.
Maturity: **built** / *partial* / (blank) = not yet.

### Proofs

| Proof | One-line verdict | Consumers | Home | Maturity |
|---|---|---|---|---|
| Event-time monotonicity trace | projected `event_time` traces monotonically to a real source column (`Traceable`/`StaticSeed`/`NotTraceable` + offset) | ba (+ cousin: as, joins) | **shared** | **built** (`monotonicity.rs`) |
| Column nullability gate | downgrade `Traceable→NotTraceable` when the leaf column is nullable/unknown | ba (+ cousin: as, joins), via the shared trace | **shared** | **built** (`smelt-db`) |
| Unified bound/reach derivation | finite backward `before` / forward `after`/`H` reach of a frame or interval band; splits *computation-reach* (derived) from *source-lateness* (declared) | ba, as | **shared** | **built** but **duplicated** (dup 1,2) → consolidate |
| Injection-point / pushdown-depth | deepest safe placement for `σ_event_time` (source / below-agg / above-window-with-lookback); eligibility = maximal pushdown depth | ba | local:ba (specialises the trace) | *partial* |
| Frame-reach taxonomy | `RANGE…INTERVAL` → derivable `k`; `ROWS`/`GROUPS`/bare `LAG`→`NotDerivable`; `UNBOUNDED`→∞ | ba | local:ba | |
| Combiner-algebra rung | ladder `monoid ⊂ decomposed-monoid ⊂ group ⊂ bounded multiset` | cu, lv, ve, as | **shared** | *partial* (cu only) — `mv` is **not** a consumer: it delegates, running no combiner; the ladder is where smelt-driven maintenance *ends* and native IVM begins |
| Inverse-free vs needs-inverse | new row folds from `(state,row)` alone, or a contribution must be *un-seen* | cu, lv, as, ba (enrichment) | **shared** | |
| Value-monotone vs order-monotone | value moves one way (`MIN`/`MAX`/`EXISTS`) vs semilattice fold whose value may switch (`MAX_BY`) | as, lv, ba (enrichment) | **shared** | |
| Join-contribution monotonicity | a semi-/dimension-join folds without an inverse and does not fan into a decrementing aggregate | as, ba-enrichment | **shared** | (new) |
| Decomposability (additive/holistic) | `SUM/COUNT/MIN/MAX/AVG` decomposable; `MEDIAN/MODE/exact-distinct` holistic — **one classifier, two consumers**: a maintained-camp *rung law*, an explicit *non-gate* for whole-partition batched (§9.3) | cu (gate), ba (non-gate) | **shared** | |
| Once-write verification | milestone contribution is idempotent/non-retracting (`NULL→set` only) | as | local:as (shares combiner allowlist) | |
| Partition alignment (scoped) | scope's `GROUP BY`/`DISTINCT` key ⊇ `partition_column`, judged per-branch/per-body — consumed with *opposite polarity*: batched *admits* on containment, cu/as *reject* on it (`*GroupByContainsPartitionColumn`) | ba, cu, as | **shared** (lands first) | **built** (`scope_*_alignment`) |
| Driving-fact / anchor resolution | among *joined* inputs, exactly-one-`Traceable` input is the driving fact/anchor (alias-scoped leaf disambiguation); note a model may *still independently window several* timeseries sources — that multi-source bound derivation is a distinct analysis | ba (joins), cu, lv, ve, as | **shared** | **built** but **duplicated** (dup 4) → consolidate |
| Fan-out / cardinality | join multiplies rows / changes target cardinality vs enriches in place | ba-joins, as-enrichment | shared | |
| Interval/temporal-join detection | bounded time band gives the second fact a finite lookback vs unbounded equi-join | ba-joins | local:ba | |
| Determinism (run vs row) + nondeterminism predicate | `NOW`/`CURRENT_*` pinnable vs `RANDOM`/`UUID` not | ba (+ the *predicate* shared with cu) | **shared** (predicate) / local:ba (taint flow) | **built** but **duplicated** (dup 3) → consolidate |
| Non-determinism taint/flow | non-deterministic value reaches only opted-in payload columns, never the skeleton | ba | local:ba | *partial* |
| Body-structure classifier | subquery **or CTE** body is transparent / group-aligned / order-sensitive (one parse-based check) | ba | local:ba | |
| Set-operation distribution | `σ` distributes over `UNION [ALL]`/`INTERSECT`/`EXCEPT` | ba | local:ba | |
| Static-seed detection | constant/NULL in the `event_time` slot → `StaticSeed`, not a stream | ba | local:ba | |
| Window-independence / ordered-execution | window reads only sources (parallelisable) vs own prior output (ordered); self-edge detection | * (orchestrator signal) | **shared** | |
| Additive-only model-diff | a model edit only *adds* columns derivable from `{existing target}∪{monotone dim}` → targeted backfill, not rebuild | * materialised | **shared** | (new) |
| Input-delta discovery | window-forward / snapshot-diff / change-feed, derived from the *source's* shape — the **proof** stage of the input-consumption axis (see thesis; pairs with the mutation-profile world-fact and the re-scan/probe transform) | * | **shared** (cross-cutting axis) | *partial* |
| Presentation-map purity | `π(state)` is a pure function of one consistent state row | cu | local:cu | |
| Run↔partition granularity alignment | `g_run ≥ g_part`, boundary-aligned (config invariant, not a SQL gate) | ba | local:ba | |
| Column-locality | output column depends only on in-window source rows vs on history outside them | ba | local:ba | |
| Event-time outer-visibility | the injected outer `WHERE event_time…` actually binds | ba | local:ba | |
| Reprocessing detection | run window overlaps already-merged partitions (double-count hazard) | cu | local:cu | |
| Batch-safety roll-up | model-level `FullyBatchSafe`/`BoundedSafe(n)`/`PerPartitionOnly` | ba | local:ba | *partial* |
| Engine-incrementalizability | backend's native IVM accepts the query (delegated gate) | mv | local:mv | |

### World-facts (declarations)

| World-fact | What it asserts | Consumers | Home | Notes |
|---|---|---|---|---|
| Timeseries clock (`event_time`/`partition`/`granularity`) | this output has a time dimension; the universal opt-in for pushdown & window-forward consumption | ba + all driving sources | **shared**, lives in core | **built** (`timeseries.md`); the biggest already-cross-cutting declaration |
| Source mutation profile (append-only / mutable / CDF) | *the* one non-derivable input-consumption fact | * | **shared**, on the source | today *inferred* from clock presence; no first-class declaration yet |
| Refresh mode (`refresh: <mode>`) | the single declared physical commitment (the *selector*) | * | **shared** | see the three-state declaration law |
| Join cardinality (`OneToOne`/`OneToMany`) | join fan-out; unknown → fail-closed `OneToMany` | ba (joins), as (enrichment) | shared | reusing for *correctness* raises the unverified-declaration stakes |
| Declared monotonicity guarantee | monotone where static proof is undecidable (UDF, large body) | ba | local:ba | may only **widen**; static default is always reject-the-push |
| Functional dependency (`key → column`) | a column is a per-key constant | as, ba (enrichment) | shared | admits `COALESCE` once-write / 1:1-after-dedup |
| Source-lateness margin | how late a source row may arrive (the declared term of the lookback split) | ba, as | **shared** | default 0; the (b) term of §8.6 |
| `nondeterministic_columns` | payload columns exempt from the determinism requirement | ba | local:ba | the one place derive-don't-declare correctly yields |
| Bounded-domain / space-budget assertion | a column's active domain is bounded → exact holistic aggregate via multiset | cu | local:cu (an *assertion* like the shared mutation profile, but only `cumulative` interprets it) | fail-loud with a cap; never the default |
| Ceiling / cost-guardrail assertion | "error if this cannot stay maintainable / exceeds this cost driver" | * | **shared** | bounds ba lookback, as `H`, keyed state cardinality alike; never changes execution |
| Migration intent | on `refresh:` change or a non-additive edit: refuse / migrate | * materialised | shared | partly derivable (additive diff), partly declared — **open** |
| Delta-driven vs idempotent-re-scan | operating point on CDF-dependency vs re-read cost | as, keyed | shared | declared or derived from source change-feed — **open** |
| Backend capability flags | `supports_native_ivm` / `supports_retraction` / `supports_merge` / `supports_insert_overwrite` | mv / keyed / ba | **shared**, on the backend | `multi_backend.md`; `supports_retraction` is blanket-backend, contrasted with per-model invertibility |

> **Freshness owner (pull vs push)** is *implied by the mode* (pull for the smelt-driven
> modes, push for `materialized_view`), not a declaration — see the law. It is what earns
> `cumulative` and `materialized_view` their *peer* names despite a shared correctness
> contract. (Deliberately not a row above: it is neither declared nor derived-from-SQL.)

### Transforms

| Transform | Mechanism | Licensed by | Consumers | Home | Maturity |
|---|---|---|---|---|---|
| Keyed monoid `merge_into` (target-as-replica) | fold delta into keyed state, never re-read history | inverse-free/monoid rung | cu, lv, ve, as, ba (enrichment) | **shared** | **built** (trait in `smelt-backend`; impls in `smelt-backend-duckdb`/`-spark`) |
| Windowed-keyed-maintenance driver | the factored step loop `classify → step over driving partitions → per-partition pushdown → create-or-merge` | driving-fact + rung | cu today; lv/ve/as prospective | **shared\*** (per-rule copy rejected in `as` spec — but built at the *first* of lv/ve/as, not before; see ordering tie-breaker) | *partial* (cu) |
| Source-filter pushdown (window-an-input) | wrap each bounded ref in a partition-column subquery | trace + bound | ba, cu, as | **shared** | **built** |
| Two-layer widened-scan + exact output clamp | scan `[start−k−offset,end)`, clamp output `[start,end)`; read the margin, never re-write it | finite frame reach `k` | ba (windows, interval joins) | local:ba | (redesign — runtime over-widens write, under-reads scan) |
| Partition DELETE+INSERT | delete+rebuild touched partitions | trace + alignment | ba | local:ba | **built** |
| Outer output-clamp | filter outermost SELECT on projected `event_time` | projection only | ba | local:ba | **built** |
| UNION-branch wrap-and-filter | per-branch source injection | per-branch partitionability | ba | local:ba | |
| Hidden decomposed state + presentation view | store `(sum,count)`/Welford/HLL, expose `π(state)` | decomposed-monoid rung | cu today; lv/as imminent | **shared\*** (single-mode *mechanism* under the imminent-2nd-consumer exception; see hoist rule) | |
| Retraction via delta history | store invertible per-partition delta; subtract-then-add | group rung | cu | local:cu | |
| Explicit multiset (bounded-domain Z-set) | per-key value→count; one state, many presentations; free retraction | bounded-domain opt-in | cu | local:cu | |
| Compile-time pinning | resolve `NOW()` once per run | run-deterministic | ba | local:ba | |
| Targeted column backfill | in-place `UPDATE`/dimension-merge for an additive diff; never a full rebuild | additive-only diff | * materialised (excl view/mv) | **shared** (schema evolution) | (new) |
| Dimension-driven MERGE into horizon-bounded target slice | merge a dimension batch straight into the target replica over `[conv_ts−H, conv_ts]`; never read the fact | target-as-replica + monotone contribution + `H` | as, ba (enrichment) | **shared** | (new) |
| Idempotent window re-scan vs delta-driven probe | CDF-free unconditional re-scan (idempotent monoid) vs per-run changed-set probe | idempotent monoid + mutation profile | as, keyed | shared | |
| Eviction / settled-key GC | retire keys older than `current_window − H`; fail-loud on cap | horizon `H` | as | local:as | |
| Watermark settled-delay / tail-rewrite | for forward reach, delay `W` until `now ≥ hi+a`, or tail-rewrite | forward reach | ba, as | shared | **open** (unworked mirror) |
| Close-old / open-new interval maintenance | SCD2 combiner over `merge_into` | order-monotone + validity | ve | local:ve | |
| Upsert-overwrite | overwrite per key (max-by-ordering monoid, or last-processed) | value/order-monotone | lv | local:lv | |
| Delegate-to-native-IVM | emit the backend's maintained object; hard error if unsupported | `supports_native_ivm` + engine gate | mv | local:mv | *partial* |
| Backfill chunking (safety-class-driven) | one shot / auto-sized chunks / per-partition | batch-safety roll-up | ba | local:ba | *partial* |
| Auto-coarsen run window | align a too-fine cadence up to `g_part` | `g_run<g_part` | ba | local:ba | |
| DAG composition (two grains) | express a mode-combo as two composed models, not a new mode | litmus rule | * | **shared** resolution mechanism | |
| Backend lowering / emulation | emulate `INSERT OVERWRITE`/create-or-replace; cross-engine `read_parquet` | capability flags | * physical emit | shared | **built** |
| Full refresh | universal fallback; the honest verdict for unmaintainable declared modes | — | `full` + fallback | shared | **built** |

## Target spec architecture

**New normative spec `docs/specs/model_maintenance.md`** owns Layer 0 and, crucially,
**owns the contract that lives today inside `cumulative_aggregate.md`**: the
processed-input equivalence invariant and the algebraic ladder. Those are currently
cited *as normative* by `versioned_models.md`, `latest_value_models.md`,
`accumulating_snapshot.md`, and `multi_backend.md` — so lifting them into
`model_maintenance.md` and rewriting those cross-references is **L0 work, not a later
"slim the specs" afterthought**; done wrong it either orphans four specs or creates a
circular re-home (cumulative → model_maintenance → cumulative). Beyond the contract,
Layer 0 owns: the three-state declaration law, the buckets-as-pipeline framing, the
input-consumption axis (reconciled with `models.md`), the shared fundamentals, and the
*litmus rule* (§19.7): contract/shape change → new *peer* mode; scan/state change →
*derived*; two contracts at two grains → *compose in the DAG*.

**Mode specs become compositions — but "thin" is uneven.** `latest_value_models.md`,
`versioned_models.md`, and `materialized_view.md` genuinely shrink to a composition
table + a short `local` residue. **`cumulative_aggregate.md` does not** — once the
contract and ladder move out, what remains is the **reference implementation** of the
shared keyed-maintenance path (retraction, reprocessing, presentation-purity) that the
other keyed modes point *at*. `accumulating_snapshot.md` likewise keeps a dense
`local` core (7-check classifier, horizon `H`, once-write provenance, hot-key GC). The
composition table is the *frame*; for the hub specs the local core is still
substantial. Each mode spec = (a) the composition table (shared proofs required,
world-facts consumed, transform driven, output shape) + (b) its `local:<mode>`
fundamentals. `models.md` keeps the axes and points at `model_maintenance.md` for the
spine.

**One drift to reconcile in L0:** `models.md` still states `accumulating_snapshot` is
not a recognized `refresh:` value and not accepted by the parser, yet a full
`accumulating_snapshot.md` spec exists. L0 must reconcile that before the composition
tables can cite it.

This is what makes `/smelt:validate` able to catch drift: a shared classifier has one
spec home, so a mode spec that silently re-describes it is flagged, not blessed.

## Target plan architecture (the re-cut master)

The re-cut master builds **bottom-up**, so every consumer is wired to an
already-shared primitive rather than a private copy:

- **L0 — Fundamentals spec.** Author `model_maintenance.md`; slim the mode specs to
  compositions. (Spec work; `/smelt:spec`.)
- **L1 — Shared proofs.** *Consolidations first* (they pay down existing debt and
  unblock everything): unified bound/reach derivation (dups 1,2), driving-fact/anchor
  resolution (dup 4), the shared nondeterminism predicate (dup 3). *Then* the new
  shared classifiers: combiner-algebra rung, inverse-free, value/order-monotone,
  join-contribution monotonicity, the scoped partition-alignment signal, decomposability
  (one classifier / two consumers), additive-diff, input-delta discovery,
  window-independence/ordered-execution.
- **L2 — Shared transforms.** Windowed-keyed-maintenance driver and presentation-view
  mechanism (both `shared*` — *back-filled* at their second consumer, not built here;
  see the ordering tie-breaker), targeted column backfill, dimension-driven
  horizon-bounded MERGE, the two-layer widened-scan/exact-clamp redesign.
- **L3 — World-fact surfaces.** Source mutation profile, functional dependency,
  source-lateness, bounded-domain assertion, ceiling guardrail, `nondeterministic_columns`,
  declared monotonicity.
- **L4 — Modes as compositions.** Each mode wires the shared proofs+transforms plus
  its `local` residue: batched (chunking, outer-clamp, column-locality, body-structure,
  set-op, static-seed, granularity/outer-visibility, taint flow); cumulative
  (retraction, multiset, reprocessing, presentation purity); latest_value (upsert);
  versioned (close/open); accumulating_snapshot (once-write, hot-key GC);
  materialized_view (delegate + engine gate).

**Ordering tie-breaker (the hoist rule dominates layer position).** Bottom-up L0→L4
is the *default* order, but it must not force a shared primitive to be built before a
second *mode* consumes it — that would reintroduce exactly the speculative generality
the hoist rule exists to prevent. When layer position and the hoist rule disagree, the
hoist rule wins: a Layer-1/2 item with only one live mode consumer is built *with the
second mode in L4* and then **back-filled** into the shared layer, not built
speculatively in its layer. This governs the `shared*` items — the windowed-keyed
driver, the presentation-view mechanism, and any classifier (inverse-free,
value/order-monotone, join-contribution) whose second mode has not yet landed. L1/L2
build *now* only the items with ≥2 live consumers: the consolidations of dups 1–6, the
monotonicity trace, the scoped partition-alignment signal, and the unified bound/reach
derivation.

### Mapping the current master onto the layers (nothing is lost)

| Current group | Re-homed as |
|---|---|
| **A** (rename/ontology) — *done* | L0 prerequisite; survives unchanged |
| **B0** filter-placement + bound derivation | L1 consolidation (dups 1,2) + injection-depth |
| **B1** UNION/subquery/join consumers | L1 (driving-fact consolidation dup 4) + local:ba (body-structure, set-op, static-seed) |
| **B2** window functions | L1 frame-reach + L2 two-layer clamp |
| **B3** non-determinism | L1 shared predicate (dup 3) + L3 `nondeterministic_columns` + local:ba taint |
| **B4** HAVING/DISTINCT | L1 partition-alignment shared signal |
| **B5** granularity alignment | local:ba + L2 auto-coarsen |
| **B6** self-referential | L1 window-independence/ordering |
| **B7** monotone-integer keys | L1 trace generalisation |
| **B8** observability | cross-cutting tooling |
| **C1/C2** decomposed monoid + view | L1 rung + L2 presentation-view |
| **C3** group-rung retraction | L1 inverse-free + L2 delta history |
| **C4** bounded-domain multiset | L3 assertion + L2 multiset state |
| **D1** latest_value | L4 composition (order-monotone + merge + driver) |
| **D2** versioned | L4 composition (driver + close/open) |
| **D3** materialized_view | L4 (delegate + capability gate) |
| monotone-join note §7/§8 | L1 join-contribution + additive-diff; L2 dimension-driven MERGE + targeted backfill (consumed by `as` **and** ba-enrichment) |

> **Mapping notes.** Group A (A1–A4, all `done`) is collapsed to one row — it is the
> ontology/rename foundation and survives intact. B8 (observability) maps to
> cross-cutting tooling, not a fundamental. Verified against the master: every phase
> A1–D3 plus the monotone-join §7/§8 row is accounted for; nothing is dropped. One
> caveat inherited from the source: the master is itself internally inconsistent (its
> registry marks Group A `done` while its phase table still lists A1–A4 `pending`);
> this note sides with the registry.

## Open questions

- **Spec granularity of `model_maintenance.md`.** One spec for all three buckets, or
  split (e.g. `maintenance_proofs.md` + `maintenance_transforms.md`)? Leaning one
  spec — the buckets are stages of one pipeline bound by a single equivalence
  contract, so a split would fragment the contract across files.
- **How much of the consolidation (dups 1–6) is a prerequisite vs opportunistic?**
  Unified bound/reach (dups 1,2) and driving-fact (dup 4) clearly gate L1; the
  interval-parser and aggregate-name dups (2,6) are cheaper and could ride along.
- **Does the windowed-keyed driver get built now, or when the second keyed mode lands?**
  The hoist rule says build it at the second consumer — cumulative is the first, so
  it lands with the first of {latest_value, versioned, accumulating_snapshot}. Decide
  in the L2 sub-plan.
- **Static/declared line for the mutation profile and additive-diff.** How much of
  append-only and "additive-only edit" is provable vs declared (mirrors the model-updates
  §18.2/§18.3 open questions).
- **Migration of in-flight `batched` code.** Group A landed against the mode-vertical
  shape; the L1 consolidations must land without regressing the existing batched
  integration tests (the per-partition equivalence oracle is the net).

## References

- Research: [`20260703-model-updates.md`](20260703-model-updates.md),
  [`20260704-monotone-join-maintenance.md`](20260704-monotone-join-maintenance.md).
- Master being re-cut: [`docs/plans/20260704-model-updates.md`](../plans/20260704-model-updates.md).
- Code (the shared primitives + the six duplications):
  `crates/smelt-logical/src/analysis/{monotonicity,source_bounds,temporal,mod}.rs`,
  `crates/smelt-logical/src/rules/{incremental,cumulative}.rs`,
  `crates/smelt-backend/src/lib.rs` (`merge_into`/DELETE+INSERT trait signatures;
  concrete impls in `smelt-backend-duckdb`/`smelt-backend-spark`),
  `crates/smelt-runtime/src/compile.rs`.
