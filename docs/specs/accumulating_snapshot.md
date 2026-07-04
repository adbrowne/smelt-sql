---
feature: accumulating_snapshot
status: experimental
last_reviewed: 2026-07-04
owners: [andrew]
---

# Accumulating Snapshot Refresh Mode

> **What this is.** A normative spec for the `refresh: accumulating_snapshot` mode — a smelt-owned **keyed-output** refresh mode for *lifecycle* / *retroactive-enrichment* facts, where each row represents an entity (an order, an event, a session) and its **milestone columns** are filled in over time as later facts arrive (`converted_at`, `order_paid_at`, `order_shipped_at`, …). One row per key, no output partition column; each milestone column is combined *once-write* / *supersede-only* across the source windows that touch its key. It is the stateful-merge counterpart of `batched` for the case where a past row must be **updated in place** by data arriving in the future, and a keyed sibling of `cumulative`, `latest_value`, and `versioned` on the refresh axis (`models.md` §"Refresh axis"). This mode is a **composition** of the maintenance framework: it names the equivalence invariant and algebraic ladder of `model_maintenance.md`, requires properties from `model_properties.md`, and drives transforms from `model_transforms.md` (§Composition). Covered here in full — the mode-local machinery: the frontmatter selector, the classifier, the milestone allowlist, the once-write / order-monotone verification, the bounded forward-attribution horizon `H`, the hot-key/settled-key cap, and the maintenance boundary as it lands in this mode. Out of scope, with their own homes: the equivalence invariant + ladder + composition contract (`model_maintenance.md`); the discriminants, join-contribution monotonicity, and functional-dependency declaration (`model_properties.md`); keyed `merge_into`, the windowed-keyed-maintenance driver, the dimension-driven horizon MERGE, source-filter pushdown, and idempotent re-scan vs delta probe (`model_transforms.md`); the running-aggregate mode (`cumulative_aggregate.md`); Type-1 / Type-2 keyed modes (`latest_value_models.md`, `versioned_models.md`); the partitioned DELETE+INSERT mode (`batched_models.md`); the source clock (`timeseries.md`) and source mutation profile (`sources.md`) this rule consumes; engine-owned maintenance (`materialized_view.md`).
>
> **Spec-first rule.** Edit this file before writing the implementation plan. The spec diff is the change description.
>
> **Timeless-oracle rule.** This spec describes the feature as if it has always existed. Implementation status lives in §Known Divergences (with plan link) or §References → Plans (history).
>
> **Status: experimental (not yet implemented).** The mode is specified ahead of implementation; `refresh: accumulating_snapshot` does not parse today — the `refresh` enum accepts only `full`/`batched`/`cumulative`/`materialized_view` (§Known Divergences). The design is worked in `docs/research/20260703-model-updates.md` Part 20 and `docs/research/20260704-monotone-join-maintenance.md`; the delivery plan is `docs/plans/20260704-accumulating-snapshot.md`.

## Surface

### Composition

Per the composition contract (`model_maintenance.md` §"The composition contract"), the mode's normative content is this table — referencing shared capabilities **by name** — plus the mode-local machinery defined in full below. It re-specifies none of the framework capabilities it names.

| Composition axis | What this mode requires / consumes / drives |
|---|---|
| **Properties required** (`model_properties.md`) | **join-contribution monotonicity** — a semi-/dimension-join's per-key contribution folds without an inverse; the **value-monotone / order-monotone** discriminants (`MIN`/`MAX`/`EXISTS` value-monotone; `MAX_BY`/`MIN_BY` order-monotone); the **functional-dependency declaration** `key → column` (admits a once-write `COALESCE`-first-non-null milestone) |
| **World-facts consumed** | the **timeseries clock** on the driving source (`partition_column`/`granularity`, `timeseries.md`); the **source mutation profile** (append-only, gating a re-scanned existence flag, `sources.md`); the **source-lateness margin** / forward horizon `H` — derived from a forward predicate where present, else declared on the source |
| **Transforms driven** (`model_transforms.md`) | keyed **`merge_into`** sequenced by the **windowed-keyed-maintenance driver**; the **dimension-driven horizon-bounded MERGE** (merge a dimension batch into the target slice `[event_ts, event_ts + H]` without re-reading the fact); **idempotent window re-scan vs delta-driven probe** for the dimension read; **source-filter pushdown** on the driving source |
| **Output shape** (`models.md` §"Refresh axis") | **keyed** — one row per `unique_key`, no `partition_column`; a **bounded forward horizon** `H` beyond which a key is settled |
| **Equivalence contract** (`model_maintenance.md`) | **end-state equivalence** — for any set `S` of processed source windows and any ordering π over `S`, stored state equals `full_refresh(model, source.where(partition ∈ S))` |

### YAML frontmatter (in `.sql` files)

```sql
---
refresh: accumulating_snapshot
---

SELECT
    event_id,
    MIN(event_ts)                                  AS occurred_at,
    MAX(conversion_ts)                             AS converted_at,
    MAX_BY(conversion_value, conversion_ts)        AS conversion_value
FROM smelt.silver.event_conversion_stream
WHERE conversion_ts IS NULL
   OR conversion_ts BETWEEN event_ts AND event_ts + INTERVAL '30 days'
GROUP BY event_id
```

`refresh: accumulating_snapshot` is the entire opt-in; it implies a stored `table` (`models.md` §Design — the modeller does not restate `materialization: table`). No rule-specific config block is read or required.

`refresh: accumulating_snapshot` **forbids** a `timeseries:` block *on the model itself* — the output is a keyed lookup with no partition column (Semantics §"Output shape"). This forbids *output* partitioning, not event-time-aware *consumption*: like `cumulative`, an accumulating-snapshot model over a source that carries a `timeseries:` declaration consumes that source window-forward (Semantics §"Windowed consumption"). It also **forbids** a `batched:` block — the two modes uphold different specialisations of the equivalence invariant on different output shapes (`model_maintenance.md` §"The equivalence invariant").

### `smelt.yml` (project-level overrides)

```yaml
models:
  event_conversions:
    refresh: accumulating_snapshot
```

Frontmatter wins over `smelt.yml` when both set `refresh`. The same forbid-`timeseries:` / forbid-`batched:` constraints apply.

### CLI

An `accumulating_snapshot` model consumes the same `--event-time-start`/`--event-time-end` flags as batched execution. The flags name the source windows whose deltas will be merged in — they apply to the **driving source's** `partition_column` / `granularity`, **not** to any column on the keyed output.

```
smelt run --event-time-start <ISO-8601> --event-time-end <ISO-8601> [selectors]
smelt backbuild --event-time-start <ISO-8601> --event-time-end <ISO-8601> [selectors]
```

A run reading driving-source rows in `[run_start, run_end)` merges into every key whose own event time lies in `[run_start − H, run_end]`, where `H` is the model's bounded forward-attribution horizon (Semantics §"The attribution horizon"). Windows may be run in any order or re-run without corrupting state (Semantics §"Overlap tolerance").

### Milestone allowlist

The classifier accepts non-key projections that are direct calls to one of the aggregators below. Each is a combiner that **folds without an inverse** (value-monotone or order-monotone — `model_properties.md` §"Algebraic discriminants"), so a new contribution merges from `(state, delta)` alone with no history re-read; each has a fixed cross-window combiner:

| Per-key aggregator | Cross-window combiner | Discriminant / meaning |
|---|---|---|
| `MIN(...)` | `LEAST` | value-monotone: earliest value per key |
| `MAX(...)` | `GREATEST` | value-monotone: latest value per key |
| `COALESCE(<col>, …)` first-non-null over the group | `COALESCE` (first non-null wins) | first observed value per key (once-write only) |
| `MAX_BY(value, ordering)` / `MIN_BY(value, ordering)` | max/min-by-ordering | order-monotone: value at the extreme of an ordering column |

The combiner column is a fixed lookup off the per-key aggregator; authors do not declare combiners (Design §"Derive the combiner").

`MIN`/`MAX`/`MIN_BY`/`MAX_BY` are admitted **unconditionally** — `LEAST`/`GREATEST`/max-by-ordering are order-independent, no-inverse folds (semilattices), so their cross-window merge converges to the full-refresh value regardless of how a key's contributions are distributed across windows. Their **reported value may switch** (a later conversion supersedes via the fold's own ordering key) — that is order-monotone and safe; what never happens is *un-seeing* a folded element (the retractable case, refused — Design §"Monotone, not retractable"). `COALESCE`-first-non-null is different: its combiner `COALESCE(target, delta)` is order-dependent *unless* a key has at most one distinct non-null value for that column, so it is admitted only when the classifier can **prove that once-write property** (Semantics §"Classifier checks"); where it cannot, the milestone fails closed with `AccumulatingSnapshotCorrectableMilestone`.

### Diagnostic codes (owned by this spec)

| Code | Severity | Trigger |
|---|---|---|
| `AccumulatingSnapshotRequiresGroupBy` | Error | The model SELECT has no `GROUP BY` — there is no unique key to derive. |
| `AccumulatingSnapshotForbidsTimeseries` | Error | The model declares both `refresh: accumulating_snapshot` and a `timeseries:` block. |
| `AccumulatingSnapshotForbidsBatched` | Error | The model declares both `refresh: accumulating_snapshot` and a `batched:` block. |
| `AccumulatingSnapshotUnknownCombiner` | Error | A non-key projection is not a direct call to a milestone aggregator in the allowlist. The diagnostic names the offending function and points at the projection. |
| `AccumulatingSnapshotCorrectableMilestone` | Error | A milestone column is not provably once-write / supersede-only (its contribution would need to be *un-seen* — a correction or delete, not a supersede). Such a column needs the group rung the mode does not provide; the diagnostic names the column and suggests `refresh: materialized_view` or `refresh: full`. |
| `AccumulatingSnapshotRetractableEnrichment` | Error | An enrichment's per-key contribution is **retractable** — it feeds a decrementing aggregate or a milestone that must be un-seen, so its join-contribution monotonicity proof fails. The diagnostic steers to `refresh: materialized_view` (native IVM) or DAG composition. It does **not** fire on the join *spelling* alone (Semantics §"Classifier checks"). |
| `AccumulatingSnapshotUnboundedHorizon` | Error | The forward-attribution horizon `H` is neither derivable from a bounded forward predicate nor declared on the source — it is unbounded, so the clamp cannot be formed. |
| `AccumulatingSnapshotGroupByContainsPartitionColumn` | Error | The `GROUP BY` list contains the driving source's `partition_column`. That produces the per-partition (batched) shape; the diagnostic suggests `refresh: batched` + `timeseries:`. |
| `AccumulatingSnapshotForbidsNondeterministic` | Error | The SQL uses `NOW()`, `RANDOM()`, or other non-deterministic functions outside stable contexts. Cross-window merge requires deterministic per-window output. |
| `AccumulatingSnapshotNoDrivingSource` | Error | No `smelt.<path>` reference in the FROM clause has a `timeseries:` declaration on the resolved target. |
| `AccumulatingSnapshotMultipleDrivingSources` | Error | More than one timeseries-tagged source appears in the FROM clause. The diagnostic lists the candidates. |

## Semantics

### Execution model

For a `refresh: accumulating_snapshot` model with a run window `[run_start, run_end)`:

1. **Classify the model's SQL** (§"Classifier checks") and derive:
   - `unique_key` — the columns named in `GROUP BY`.
   - `milestone_columns` — a map from each non-key projection's output column name to its `(per_key_aggregator, cross_window_combiner)` pair, from the Surface §"Milestone allowlist" table.
   - `driving_source` — the single timeseries-tagged source in the FROM clause (§"Driving source").
   - `H` — the bounded forward-attribution horizon (§"The attribution horizon").
2. **Drive the windowed-keyed-maintenance driver** (`model_transforms.md`). It steps over the driving-source windows covered by `[run_start, run_end)` in temporal order. For each window `W`:
   - **Source-filter pushdown** injects `<driving_source>.<partition_column> >= W AND < W + granularity` onto the driving source's reference. Non-timeseries sources (lookups / dimensions) are read in full each window, subject to the dimension read plan below.
   - **Execute the per-window delta SELECT**, producing one delta row per `unique_key` value present in this window's input, with its milestone columns.
   - **Backend `merge_into` call** with the derived `unique_key` and the per-column combiner map. Matched keys: each milestone column is combined via its cross-window combiner (`LEAST(target.occurred_at, delta.occurred_at)`, `GREATEST(target.converted_at, delta.converted_at)`, `COALESCE(target.first_touch, delta.first_touch)`). Unmatched keys: insert as-is.
3. If the output table does not exist when the first window is merged, the driver creates it from that window's delta SELECT (`CREATE TABLE AS SELECT`); subsequent windows merge into it.

There is **no** DELETE and **no** partition rebuild. The write touches only the keys present in each window's delta — the sparse-update property that distinguishes this mode from batched (Design §"Keyed MERGE, not DELETE+INSERT").

A **conversion-driven** enrichment (a dimension of conversions arriving to fill milestones on already-stored events) uses the **dimension-driven horizon-bounded MERGE** (`model_transforms.md`): the conversion batch merges directly into the target slice `[event_ts, event_ts + H]` **without re-reading the fact**. The dimension read is either a **delta-driven probe** (when the dimension source exposes a change feed) or an **idempotent window re-scan** (when it does not and the fold is idempotent) — the idempotent re-scan vs delta probe transform, selected from the dimension source's mutation profile, not a per-model knob.

### Output shape

An accumulating-snapshot model's output has:

- One row per `unique_key` value (the `GROUP BY` column list).
- Milestone columns whose values reflect the once-write / supersede-only combine of every processed source window's contribution to that key. A milestone not yet observed for a key is `NULL`.
- **No** `partition_column`. **No** `event_time_column`. **No** `timeseries:` declaration on the model itself.

Downstream consumers see the output as a keyed lookup table — there is no partition information to push down; they read it in full each run, identical to any non-timeseries source.

### Driving source

The classifier walks the inlined outer SELECT's FROM clause (after function expansion, per `expansion.md`) and collects every `smelt.<path>` reference whose resolved target declares a `timeseries:` block. The result must be exactly one — the **driving source** (the anchor, resolved via driving-fact / anchor resolution, `model_properties.md`), whose `partition_column` and `granularity` parameterise the per-window step loop, the source-filter pushdown, and the run-window clamp.

| Cardinality of timeseries-tagged sources | Outcome |
|---|---|
| 0 | Rejected: `AccumulatingSnapshotNoDrivingSource`. |
| 1 | Accepted. |
| ≥ 2 | Rejected: `AccumulatingSnapshotMultipleDrivingSources`. |

Non-timeseries sources in the FROM clause (dimensions / lookups) are allowed. A *fact-to-dimension join* that brings the enriching event in as a separately-arriving relation is **admitted when its per-key contribution is provably monotone** (Design §"Monotone, not retractable"), not rejected on syntax.

### The attribution horizon

The horizon `H` is the maximum time by which an enriching fact may arrive *after* the event it enriches. It bounds the run-window clamp: a run over `[run_start, run_end)` may only touch keys whose event time is `≥ run_start − H`. `H` is a **watermark-style completeness bound**: an enriching fact arriving more than `H` after its event is **dropped** (the key is considered settled), unless a full refresh is run.

`H` must be **bounded**. It is resolved in one of two ways (the derived / declared split of `model_properties.md` §"Unified bound / reach derivation" — the `after` forward reach):

1. **Derived** — when the model expresses the attribution window as a forward predicate on the driving source, e.g. `conversion_ts BETWEEN event_ts AND event_ts + INTERVAL '30 days'`. The `+ INTERVAL '30 days'` is a forward reach; `H = 30 days` is read from the SQL (derive-don't-declare). This is the preferred form.
2. **Declared** — on the *source*, as a source-lateness property shared by every consumer (the `timeseries:` source declaration, default `0`). Used when the horizon is a pipeline property not expressible as a SQL predicate.

An **unbounded** horizon — no forward predicate and no source-lateness declaration that bounds it — is rejected (`AccumulatingSnapshotUnboundedHorizon`): with `H → ∞` the clamp `run_start − H → −∞`, every run would touch all history and retain unbounded hot state. This is the exact mirror of `batched` rejecting an `UNBOUNDED PRECEDING` lookback frame.

There is **no per-model horizon override**. `H` is either read from the model's own SQL (derived) or inherited from the driving source's declaration (a world-fact shared by every consumer of that source). A per-model override that diverged from the source's declared lateness would let one consumer claim a completeness bound the source does not honour; the two resolution paths above are the whole surface.

### Classifier checks

A `refresh: accumulating_snapshot` model is rejected at planning time if any of these hold on the inlined outer SELECT (after function expansion):

1. **No `GROUP BY`** — `AccumulatingSnapshotRequiresGroupBy`.
2. **Non-key projection is not an allowlisted milestone aggregator** — `AccumulatingSnapshotUnknownCombiner`. Composite expressions over aggregates are rejected; add columns for the underlying milestones and derive downstream.
3. **A milestone is not provably once-write / supersede-only** — `AccumulatingSnapshotCorrectableMilestone`. This check applies **only to `COALESCE`-first-non-null milestones**: `MIN`/`MAX`/`MIN_BY`/`MAX_BY` are no-inverse folds whose merge converges regardless of once-write, so they need no proof (Surface §"Milestone allowlist"). A `COALESCE`-first-non-null milestone is once-write — at most one distinct non-null value per key — and therefore admitted, only when the classifier can prove it via one of a **bounded set of provable forms**:
   - **Key-derived** — the `COALESCE` argument is a function of the `GROUP BY` key alone (trivially constant per key).
   - **Source-declared functional dependency** — the driving source declares `key → column` (the functional-dependency declaration of `model_properties.md`; the value is a per-key constant by the source's own contract).

   Any `COALESCE`-first-non-null milestone the classifier cannot place in one of these forms **fails closed** — the once-write provenance analysis is deliberately conservative, refusing rather than assuming a value is per-key constant. (`MIN`/`MAX`/`MIN_BY`/`MAX_BY` are never rejected by this check.)
4. **An enrichment's join contribution is retractable** — `AccumulatingSnapshotRetractableEnrichment`. The classifier runs **join-contribution monotonicity** (`model_properties.md`) on any fact-to-dimension join: a semi-/dimension-join whose per-key contribution folds without an inverse (feeds only value-monotone or order-monotone milestones, does not fan into a decrementing aggregate) is **admitted** — the join spelling is normalised to the same keyed-monoid merge as the union spelling. Only the genuinely **retractable** contribution — one that feeds a decrementing aggregate or a milestone that must be un-seen — is rejected (Design §"Monotone, not retractable"). A re-scanned existence flag additionally requires the dimension source to be declared **append-only** (`sources.md`); extremal milestones are safe regardless.
5. **`GROUP BY` contains the driving source's `partition_column`** — `AccumulatingSnapshotGroupByContainsPartitionColumn` (that is the batched shape).
6. **Non-deterministic functions in the outer body** — `AccumulatingSnapshotForbidsNondeterministic`.
7. **Unbounded forward horizon** — `AccumulatingSnapshotUnboundedHorizon` (§"The attribution horizon").

There is **no** `safety_overrides:` block. The rejected constructs break the equivalence invariant (correctable milestones need retraction; retractable enrichment needs the group rung / native IVM), not merely a partial-correctness property — there is no bypass (Design §"No safety_overrides").

### Once-write end-state equivalence

This mode upholds the **end-state equivalence** specialisation of the framework invariant (`model_maintenance.md` §"The equivalence invariant"), specialised **per milestone column**:

```
accumulating_snapshot_run(model, π(S))  ==  full_refresh(model, source.where(partition_col ∈ S))
```

For any set of driving-source windows `S = {W₁, …, Wₙ}` and any ordering π over `S`, the stored value of each milestone column for each key equals what a full refresh would compute over the same set of source windows, independent of the order (or overlap) in which they were processed. This holds because every milestone combiner is a **commutative, associative, idempotent** no-inverse fold with identity `NULL` — so `LEAST`/`GREATEST`/`COALESCE`/max-by-ordering over any ordering, with repeats, converge to the same value.

### The maintenance boundary

On the algebraic ladder (`model_maintenance.md` §"The algebraic maintenance ladder"), the milestone combiners sit on the **direct-monoid** rung and no higher:

- **Direct monoid — the whole of this mode.** `LEAST`/`GREATEST`/`COALESCE`/`MAX_BY` are the value-monotone / order-monotone no-inverse folds of `model_properties.md`, `merge_into`-maintainable on a plain engine with no native IVM. Because they are additionally **idempotent**, re-merging an already-applied window is a no-op (§"Overlap tolerance") — a stronger property than `cumulative`'s non-idempotent `SUM`/`COUNT`.
- **Not a group.** These combiners are monoids but **not** groups: you cannot un-see a folded element without the underlying multiset. A milestone that must be *retracted* (corrected or deleted, not merely superseded) is therefore out of scope and fails closed (`AccumulatingSnapshotCorrectableMilestone` / `AccumulatingSnapshotRetractableEnrichment`). Retraction is the group rung, delegated to engine-native IVM via `refresh: materialized_view`.

### Windowed consumption

Batch-by-batch consumption is **not** a property of this mode — it is the input-consumption axis (`models.md`), orthogonal to the refresh mode and **derived** from the driving source's shape:

- Because the driving source carries a `timeseries:` clock, the model is consumed **window-forward**: the `--event-time` run window applies to the *source's* `partition_column`, only the new tail is read, and the run steps over covered windows in temporal order (exactly as `cumulative` consumes its driving source).
- The **clamp** falls out of the horizon: a window `[run_start, run_end)` merges into keys with event time `≥ run_start − H`. State kept *hot* (mergeable) is only keys within `H` of the current window; older keys are settled.

Windowed consumption is not selected by any knob — there is no `strategy:` selector and no window field on the model (Design §"One declaration").

### Overlap tolerance

Because the milestone combiners are idempotent no-inverse folds, run windows may be processed **out of order, backfilled in slices, or run in parallel**, and any window may be **re-run**, without corrupting state. Re-merging an already-applied window converges to the same value (`GREATEST(x, GREATEST(x, y)) = GREATEST(x, y)`). The mode therefore needs **no** precise DELETE-covers-INSERT write-window invariant — there is no DELETE, and the clamp is a *work* bound (which keys are eligible to be touched), not a *correctness* bound.

### The hot-key set and its space cap

The keys eligible to be merged in a given run — those with event time `≥ run_start − H` (the clamp) — are the **hot set**. Keys older than that are **settled**: no future in-window delta can reach them.

This mode does **not** garbage-collect settled keys from the stored table — the output is a full keyed lookup and every key, hot or settled, remains readable. What is bounded is the *work*: only hot keys are candidates for a `merge_into` on any run. The stored table therefore grows with the total key space, exactly as a full-refresh table would; the clamp bounds only how far back a run reaches, not the table size.

To keep the mode fail-loud (`architecture.md` §"Fail-loud discipline"), the rule asserts a **cap on the number of keys touched in a single run's merge** (the per-run hot-key working set). If a run's delta would merge more keys than the cap, the rule **errors** — it does not silently process an unbounded working set — and the diagnostic steers the operator to narrow the run window or run a full refresh. The cap is a coarse guard against a mis-derived or unbounded-in-practice horizon, not a correctness mechanism; the concrete default and whether it is operator-tunable are settled at implementation time (§Known Divergences).

### Functions inside bodies

Function expansion (`expansion.md`) runs **before** the classifier. Milestone-projection reading, GROUP-BY inspection, FROM-clause walking, horizon derivation, and pushdown operate on the expanded CST. A `smelt.define`-resolved milestone aggregator is admitted only if its expanded body produces an allowlisted no-inverse combiner at the outermost expression position; opaque calls (`smelt.extern`, non-inlinable built-ins) are rejected via `AccumulatingSnapshotUnknownCombiner`.

### Interaction with `--auto` / staleness

`--auto`'s staleness analysis for an accumulating-snapshot model re-processes the stale driving-source windows: any window whose input changed is re-stepped, and because the combiners are idempotent, re-running it is always safe (unlike `cumulative`'s reversible-aggregator caveat) — no widening of the read window is required, since `merge_into` only touches the keys present in each re-run window's deltas. The precise staleness *fidelity* — whether `--auto` re-runs exactly the changed windows or conservatively re-runs from the earliest stale point — is tied to the eviction/settled-key decision (§Known Divergences).

### `unique_key` and column naming

The rule derives `unique_key` from `GROUP BY`. Output column names are the projection list's `AS` aliases (or source column names). `MAX(conversion_ts) AS converted_at` produces a `converted_at` column holding the latest observed conversion time per key across all merged windows.

## Design

**Keyed MERGE, not DELETE+INSERT.** Retroactive enrichment updates a *small fraction* of past rows (only keys that gained a milestone this window) and may reach *far* back (an event converts many days later). Batched's whole-partition DELETE+INSERT would rebuild every row of a touched partition to update a handful — the wrong write primitive. This mode drives the keyed `merge_into` transform (`model_transforms.md`): touch only the keys in the delta, combine per column. *Modelling enrichment as a batched model with a widened outer clamp* was rejected because the reach is sparse and potentially long — batched rebuild cost scales with partition size, not with the number of changed rows.

**Derive the combiner from the SQL.** `GROUP BY` names the key; each milestone projection names its aggregator; the cross-window combiner is a fixed lookup (`MIN → LEAST`, `MAX → GREATEST`, `COALESCE → COALESCE`). *A `milestones:` config block listing columns and combiners* was rejected for the same reason `cumulative` rejects an `aggregators:` block: it re-introduces the metadata-vs-SQL drift the maintained family exists to avoid. If it is in the SQL, it is not also in YAML.

**Bounded horizon, or refuse.** The clamp — and therefore batch-by-batch consumption and bounded hot state — exists only if the forward reach `H` is finite. An unbounded horizon collapses the clamp and forces full-history scans with unbounded state. Rather than silently degrade, the classifier refuses (`AccumulatingSnapshotUnboundedHorizon`), the mirror of batched refusing `UNBOUNDED PRECEDING`. `H` is derived from a `BETWEEN … + INTERVAL` forward predicate where present (the `after` half of the reach derivation, `model_properties.md`) and declared on the source otherwise — cleanly separating *computation reach* (derivable) from *source lateness* (a world-fact that must be declared).

**Monotone, not retractable — the boundary is semantics, not syntax.** The maintainability line is drawn at **monotone-vs-retractable semantics**, not join-vs-union *spelling* (settled in `docs/research/20260704-monotone-join-maintenance.md` §§3,10). A join whose per-key contribution folds without an inverse — feeding only value-monotone (`MIN`/`MAX`/first-non-null/`EXISTS`) or order-monotone (`MAX_BY` under a *data* ordering key) milestones, with no decrementing fan-out — is semantically identical to the keyed-union form and is `merge_into`-maintainable on a plain engine with **no native IVM**. So `events LEFT JOIN conversions` is admitted when the classifier's **join-contribution monotonicity** proof passes; the join spelling is a front-end normalisation to the one keyed-monoid merge, not a second execution path. Only the genuinely **retractable** contribution — a `COUNT(conversions)` that must decrement, a value that must be un-seen because a folded element was corrected or deleted — is refused (`AccumulatingSnapshotRetractableEnrichment`) and routed to `refresh: materialized_view` or DAG composition. *The earlier syntactic reject of every join* was rejected as one notch too conservative: it forced a mechanical union rewrite of the common, safe case.

**Native IVM is the retractable-slice fallback, not the enrichment answer.** Delegating the enrichment join to engine-native IVM everywhere was rejected. Trusting an engine's incremental-view runtime on large tables is exactly where a smelt-driven MERGE earns its existence: native IVM is opaque and can strand a pipeline in a full-table rebuild with no partial-recovery escape hatch, whereas a smelt-driven `merge_into` over plain tables always has one. So `refresh: materialized_view` is the fallback for the *retractable* slice smelt genuinely cannot self-maintain, not the primary path for monotone enrichment. Whether the modeller writes the union spelling or the (monotone) join spelling, the classifier normalises to the same keyed-monoid merge — the union is not privileged, only the monotone-contribution requirement is.

**One declaration, everything else derived.** The *contract* (`refresh: accumulating_snapshot`) is declared; the *scan* (window-forward, batch-by-batch, clamped) is derived from the driving source carrying a `timeseries:` clock, exactly as for `cumulative`. *A `strategy:` selector or a `batched_snapshot:` mode* was rejected as the dbt-`incremental_strategy` anti-pattern the maintained family is designed against — a knob that silently changes the contract (`model_maintenance.md` §"Validator, not chooser").

**No `safety_overrides`.** Batched exposes `safety_overrides:` because some of its rejections guard a *partial-correctness* optimisation the modeller may knowingly waive. This mode's rejections instead guard the *equivalence invariant itself* — a correctable milestone genuinely needs retraction, a retractable enrichment genuinely needs the group rung — so there is nothing safe to waive. The escape from a rejection is to remodel (bounded predicate, monotone contribution) or move to `refresh: materialized_view`, not to override.

**Output is not a timeseries.** One row per key, no partition column; the model forbids `timeseries:` on itself and reads its partition shape from the driving source. Downstream consumers treat it as a lookup. This is the same boundary `cumulative`, `latest_value`, and `versioned` draw.

**One windowed executor, shared with `cumulative`.** The window-forward step loop is the **windowed-keyed-maintenance driver** (`model_transforms.md`) — a mode-agnostic mechanism parameterised by `(classifier, merge-SQL builder)`. Accumulating snapshot does **not** copy `cumulative`'s executor; both modes (and, prospectively, `latest_value` / `versioned`) consume the one driver. *A per-rule copy of the step loop* was rejected as the drift risk of four near-identical clamp/inject/merge loops. A consequence: the mode inherits the shared driver's granularity support (`cumulative`'s `day`/`week` today); widening granularities is a property of the shared driver, not of this mode.

## Constraints & Invariants

1. **Opt-in is `refresh: accumulating_snapshot` alone** (storage implied `table`). No rule-specific config block.
2. **`timeseries:` and a `batched:` block are forbidden on the model.** Diagnostics `AccumulatingSnapshotForbidsTimeseries`, `AccumulatingSnapshotForbidsBatched`.
3. **`unique_key` is derived from `GROUP BY`.** No `GROUP BY` → `AccumulatingSnapshotRequiresGroupBy`.
4. **Milestone combiners are a fixed lookup off the projection aggregator.** Authors do not declare combiners.
5. **Every milestone combiner is a commutative, associative, idempotent no-inverse fold** (a monoid, not a group). Retractable milestones are refused.
6. **Every milestone is once-write or supersede-only** (never un-sees a folded element). Correction/retraction is out of scope.
7. **The forward-attribution horizon `H` is bounded** — derived from a forward predicate or declared on the source. Unbounded → refused.
8. **The run-window clamp is `run_start − H`** on the driving source's clock; it never depends on data values (a function of the run window and `H` only).
9. **The maintainability boundary is monotone-vs-retractable, not join-vs-union.** A join with a provably monotone per-key contribution is admitted; only a retractable contribution is refused (`AccumulatingSnapshotRetractableEnrichment`).
10. **End-state equivalence holds for any ordering and any overlap** of processed windows (idempotent no-inverse fold).
11. **No `partition_column` on the output.** Downstream treats it as a lookup.
12. **No silent downgrade.** A classifier rejection refuses the model at planning time — no fallback to batched, to full-refresh, or to `materialized_view` (`model_maintenance.md` §"Validator, not chooser").
13. **The per-run hot-key working set is capped and fail-loud.** A run whose delta would merge more keys than the cap errors rather than processing an unbounded working set. Settled keys are never GC'd from the stored table.
14. **The windowed step loop is shared, not copied.** Accumulating snapshot consumes the same windowed-keyed-maintenance driver as `cumulative` (`model_transforms.md`).
15. **`COALESCE`-first-non-null milestones require a once-write provenance proof; `MIN`/`MAX`/`MIN_BY`/`MAX_BY` do not.** Unprovable `COALESCE` milestones fail closed.
16. **`H` has no per-model override** — derived from the model SQL or declared on the driving source, nothing else.

## Known Divergences / Open Questions

- **Not implemented; the mode does not parse.** `refresh: accumulating_snapshot` currently produces an invalid-refresh-strategy error — `RefreshStrategy::Deserialize` in `crates/smelt-core/src/config.rs` accepts only `full`/`batched`/`cumulative`/`materialized_view`. The enum value, the classifier, the milestone-combiner derivation, the join-contribution-monotonicity check, and the windowed `merge_into` execution are all unbuilt. The design is worked in `docs/research/20260703-model-updates.md` Part 20 and `docs/research/20260704-monotone-join-maintenance.md`; the delivery plan is `docs/plans/20260704-accumulating-snapshot.md`.
- **The forward-reach walk `H` consumes exists (`source_bounds.rs`); the accumulating-snapshot classifier that reads it is unbuilt.** A derived horizon from a `BETWEEN … + INTERVAL` forward predicate reads the `after` half of the source bound (`model_properties.md` §"Unified bound / reach derivation"). Wiring it into this mode's own classifier is scope for the delivery plan.
- **Join-contribution monotonicity is `not-yet` in `model_properties.md`.** The classifier check that admits the monotone join spelling depends on a property that is specified but unbuilt. Where the static/declared line falls — how much monotonicity is statically provable vs needs a declaration, and whether 1:1-after-dedup is recognised structurally or via declared join cardinality — is open (`docs/research/20260704-monotone-join-maintenance.md` §"Open questions").
- **The hot-key cap default is unspecified.** The concrete default cap and whether it is operator-tunable are settled at implementation time. A value, not a design fork.
- **Settled-key GC is a deferred enhancement, not v1.** v1 keeps every key (hot and settled) in the stored lookup and never garbage-collects — the table grows with the total key space, as a full-refresh table would. A future space-budget GC that retires keys older than `run_window − H` from a *hot-state store* is possible if a persistent-watermark store is added, but v1 needs none: the clamp already bounds the per-run work, and the fail-loud cap guards the working set. Eviction / settled-key GC is the mode-local transform (`model_transforms.md` §"Transforms that stay in a mode spec").
- **`COALESCE` once-write provenance breadth may widen.** v1 admits a `COALESCE`-first-non-null milestone only via the two provable forms (key-derived; source-declared functional dependency), failing closed otherwise. Broadening the provable set is a future refinement. `MIN`/`MAX`/`MIN_BY`/`MAX_BY` are unaffected — they need no proof.
- **Non-determinism run-pinning is a deferred alignment.** v1 rejects `NOW()`/`RANDOM()` outright. Adopting the compile-time run-pinning transform for `NOW`/`CURRENT_*` (`model_transforms.md`, once proven in `batched`) is a later step; the conservative reject holds until then.
- **Sibling keyed modes.** `cumulative` (running aggregate), `latest_value` (Type 1), and `versioned` (Type 2) are peers on the refresh axis with their own specs and classifiers, not variants of this rule. They share the windowed-keyed-maintenance driver this mode also consumes. The engine-maintained counterpart of any keyed mode is `refresh: materialized_view`, not a maintainer flag here.

## References

- **Code**:
  - `crates/smelt-core/src/config.rs` — the `refresh` axis enum (`RefreshStrategy`); the `accumulating_snapshot` value is not yet a variant (§Known Divergences)
  - `crates/smelt-core/src/metadata.rs` — frontmatter extraction; validation that `timeseries:` / `batched:` are absent when `refresh: accumulating_snapshot`
  - `crates/smelt-logical/src/rules/` — host for the accumulating-snapshot classifier (pure rule-data in `smelt-logical`; see `architecture.md` §"Layered single-ownership")
  - `crates/smelt-logical/src/analysis/source_bounds.rs` — the forward-reach (`after`) derivation the horizon consumes; `monotonicity.rs` `trace_event_time` — the join-contribution-monotonicity classifier is a cousin of it
  - `crates/smelt-runtime/src/cumulative.rs` (→ generalising into the shared windowed-keyed-maintenance driver) — the window-forward step loop this mode shares with `cumulative`
  - `crates/smelt-backend/src/lib.rs` — `merge_into` trait method (physical primitive the rule drives); impl in `crates/smelt-backend-duckdb/src/lib.rs`
- **Tests**: accumulating-snapshot classifier unit tests and once-write / end-state equivalence tests (to be added alongside the delivery plan)
- **User docs**: `docs-site/docs/guide/materializations.md` (to document `refresh: accumulating_snapshot` on the refresh axis, alongside `batched` and `cumulative`)
- **Plans (history)**:
  - `docs/plans/20260704-accumulating-snapshot.md` — this mode's delivery plan (classifier, shared windowed executor, combiner + once-write prover, join-contribution monotonicity, horizon, hot-key cap)
  - `docs/plans/20260704-model-updates.md` — the mode-vertical master that re-cuts the maintenance framework this mode composes
- **Related specs**:
  - `model_maintenance.md` — the equivalence invariant, the algebraic ladder, the composition contract, validator-not-chooser
  - `model_properties.md` — the discriminants, join-contribution monotonicity, driving-fact resolution, functional-dependency declaration, the reach derivation
  - `model_transforms.md` — keyed `merge_into`, the windowed-keyed-maintenance driver, the dimension-driven horizon MERGE, source-filter pushdown, idempotent re-scan vs delta probe, eviction/settled-key GC
  - `cumulative_aggregate.md` — the running-aggregate keyed peer; reference implementation of the keyed-maintenance path
  - `latest_value_models.md`, `versioned_models.md` — the other smelt-maintained keyed modes
  - `materialized_view.md` — engine-native IVM; the retractable-slice fallback for correctable milestones and retractable enrichment
  - `batched_models.md` — the partitioned-output peer whose lookback clamp this mode mirrors on the driver axis
  - `timeseries.md`, `sources.md` — the source clock and mutation profile this rule consumes
  - `models.md` — the three axes; refresh-axis host; input-consumption axis; declaration law and litmus rule
  - `expansion.md` — function expansion; runs before the classifier
  - `architecture.md` — `smelt.<path>` addressing; backend primitive contract; layered single-ownership; fail-loud
- **Research**: `docs/research/20260704-monotone-join-maintenance.md` (the monotone-vs-retractable boundary §§3,10; native IVM as the retractable-slice fallback); `docs/research/20260703-model-updates.md` (Part 20 worked design; §14 monoid/group ladder; §19 input-consumption axis; §8 forward reach + watermark bound)
