---
feature: accumulating_snapshot
status: experimental
last_reviewed: 2026-07-04
owners: [andrew]
---

# Accumulating Snapshot Refresh Mode

> **What this is.** A normative spec for the `refresh: accumulating_snapshot` mode — a smelt-owned **keyed-output** refresh mode for *lifecycle* / *retroactive-enrichment* facts, where each row represents an entity (an order, an event, a session) and its **milestone columns** are filled in over time as later facts arrive (`converted_at`, `order_paid_at`, `order_shipped_at`, …). One row per key, no output partition column; each milestone column is combined *once-write* across the source windows that touch its key. It is the stateful-merge counterpart of `batched` for the case where a past row must be **updated in place** by data arriving in the future, and a keyed sibling of `cumulative`, `latest_value`, and `versioned` on the refresh axis (`models.md` §"Refresh axis"). Covers the frontmatter selector, the classifier, the milestone-combiner derivation, the bounded forward-attribution horizon, the windowed-consumption behaviour, the end-state equivalence contract, and the maintenance boundary. Out of scope: the running-aggregate mode (`cumulative_aggregate.md`); Type-1 / Type-2 keyed modes (`latest_value_models.md`, `versioned_models.md`); the partitioned DELETE+INSERT mode (`batched_models.md`); the `timeseries:` declaration this rule consumes from its driving source (`timeseries.md`); the backend `merge_into` primitive (`architecture.md` §"Backend primitives" — this rule is one caller); engine-owned maintenance (`materialized_view.md`).
>
> **Spec-first rule.** Edit this file before writing the implementation plan. The spec diff is the change description.
>
> **Timeless-oracle rule.** This spec describes the feature as if it has always existed. Implementation status lives in §Known Divergences (with plan link) or §References → Plans (history).
>
> **Status: experimental (not yet implemented).** The mode is specified ahead of implementation; `refresh: accumulating_snapshot` currently produces an unknown-refresh-value error. The design is worked in `docs/research/20260703-model-updates.md` Part 20; the delivery plan is `docs/plans/20260704-accumulating-snapshot.md`. The sole new *engine* dependency (the `after_secs` forward-reach derivation the horizon consumes) is landed by `docs/plans/20260704-model-updates-group-b.md` (phase B2); the classifier, combiner derivation, and windowed `merge_into` execution are the delivery plan's own scope.

## Surface

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

`refresh: accumulating_snapshot` **forbids** a `timeseries:` block *on the model itself* — the output is a keyed lookup with no partition column (Semantics §"Output shape"). This forbids *output* partitioning, not event-time-aware *consumption*: like `cumulative`, an accumulating-snapshot model over a source that carries a `timeseries:` declaration consumes that source window-forward (Semantics §"Windowed consumption"). It also **forbids** a `batched:` block — the two modes uphold different equivalence contracts on different output shapes (`batched_models.md`).

### `smelt.yml` (project-level overrides)

```yaml
models:
  event_conversions:
    refresh: accumulating_snapshot
```

Frontmatter wins over `smelt.yml` when both set `refresh`. The same forbid-`timeseries:` / forbid-`batched:` constraints apply.

### CLI

An `accumulating_snapshot` model consumes the same `--event-time-start`/`--event-time-end` flags as batched execution. The flags name the source windows whose deltas will be merged in — they apply to the **driving source's** `partition_column` / `granularity` (Semantics §"Driving source"), **not** to any column on the keyed output (the spelling `cumulative_aggregate.md` §CLI uses).

```
smelt run --event-time-start <ISO-8601> --event-time-end <ISO-8601> [selectors]
smelt backbuild --event-time-start <ISO-8601> --event-time-end <ISO-8601> [selectors]
```

A run reading driving-source rows in `[run_start, run_end)` merges into every key whose own event time lies in `[run_start − H, run_end]`, where `H` is the model's bounded forward-attribution horizon (Semantics §"The attribution horizon"). Windows may be run in any order or re-run without corrupting state (Semantics §"Overlap tolerance").

### Milestone combiner allowlist

The classifier accepts non-key projections that are direct calls to one of the *once-write* / extremal aggregators below. Each has a fixed cross-window combiner and a proof obligation that its per-key contribution only ever transitions a column from absent to set:

| Per-key aggregator | Cross-window combiner | Once-write meaning |
|---|---|---|
| `MIN(...)` | `LEAST` | earliest value per key |
| `MAX(...)` | `GREATEST` | latest value per key |
| `COALESCE(<col>, …)` first-non-null over the group | `COALESCE` (first non-null wins) | first observed value per key |
| `MAX_BY(value, ordering)` / `MIN_BY(value, ordering)` | max/min-by-ordering | value at the extreme of an ordering column (§19.4 monoid form) |

The combiner column is a fixed lookup off the per-key aggregator; authors do not declare combiners (Design §"Derive the combiner"). Any other aggregate, any non-aggregate non-key expression, or any milestone that can be *revised* rather than *filled once* is rejected (Semantics §"Classifier checks").

`MIN`/`MAX`/`MIN_BY`/`MAX_BY` are admitted **unconditionally** — `LEAST`/`GREATEST`/max-by-ordering are order-independent monoids, so their cross-window merge converges to the full-refresh value regardless of how a key's contributions are distributed across windows. `COALESCE`-first-non-null is different: its combiner `COALESCE(target, delta)` is order-dependent *unless* a key has at most one distinct non-null value for that column. It is therefore admitted only when the classifier can **prove that once-write property** (Semantics §"Classifier checks", once-write provenance); where it cannot, the milestone fails closed with `AccumulatingSnapshotCorrectableMilestone`.

### Diagnostic codes (owned by this spec)

| Code | Severity | Trigger |
|---|---|---|
| `AccumulatingSnapshotRequiresGroupBy` | Error | The model SELECT has no `GROUP BY` — there is no unique key to derive. |
| `AccumulatingSnapshotForbidsTimeseries` | Error | The model declares both `refresh: accumulating_snapshot` and a `timeseries:` block. |
| `AccumulatingSnapshotForbidsBatched` | Error | The model declares both `refresh: accumulating_snapshot` and a `batched:` block. |
| `AccumulatingSnapshotUnknownCombiner` | Error | A non-key projection is not a direct call to a milestone aggregator in the allowlist. The diagnostic names the offending function and points at the projection. |
| `AccumulatingSnapshotCorrectableMilestone` | Error | A milestone column is not provably once-write (it can transition set → different-set, i.e. be revised). Such a column needs a group/retraction the mode does not provide; the diagnostic names the column and suggests `refresh: materialized_view` or `refresh: full`. |
| `AccumulatingSnapshotJoinExpressedEnrichment` | Error | The enrichment is expressed as a fact-to-dimension join rather than a keyed union over one driving stream. The diagnostic steers to a keyed-union model or full refresh. |
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
   - `milestone_columns` — a map from each non-key projection's output column name to its `(per_key_aggregator, cross_window_combiner)` pair, from the Surface §"Milestone combiner allowlist" table.
   - `driving_source` — the single timeseries-tagged source in the FROM clause (§"Driving source").
   - `H` — the bounded forward-attribution horizon (§"The attribution horizon").
2. **Step over source windows** in temporal order. For each driving-source window `W` covered by `[run_start, run_end)`:
   - **Source-filter pushdown** injects `<driving_source>.<partition_column> >= W AND < W + granularity` onto the driving source's reference. Non-timeseries sources are read in full each window.
   - **Execute the per-window delta SELECT**, producing one delta row per `unique_key` value present in this window's input, with its milestone columns.
   - **Backend `merge_into` call** with the derived `unique_key` and the per-column combiner map. Matched keys: each milestone column is combined via its cross-window combiner (`LEAST(target.occurred_at, delta.occurred_at)`, `GREATEST(target.converted_at, delta.converted_at)`, `COALESCE(target.first_touch, delta.first_touch)`). Unmatched keys: insert as-is.
3. If the output table does not exist when the first window is merged, the rule creates it from that window's delta SELECT (`CREATE TABLE AS SELECT`); subsequent windows merge into it.

There is **no** DELETE and **no** partition rebuild. The write touches only the keys present in each window's delta — the sparse-update property that distinguishes this mode from batched (Design §"Keyed MERGE, not DELETE+INSERT").

### Output shape

An accumulating-snapshot model's output has:

- One row per `unique_key` value (the `GROUP BY` column list).
- Milestone columns whose values reflect the once-write combine of every processed source window's contribution to that key. A milestone not yet observed for a key is `NULL`.
- **No** `partition_column`. **No** `event_time_column`. **No** `timeseries:` declaration on the model itself.

Downstream consumers see the output as a keyed lookup table — there is no partition information to push down; they read it in full each run, identical to any non-timeseries source (`batched_models.md` §"Source-filter pushdown").

### Driving source

The classifier walks the inlined outer SELECT's FROM clause (after function expansion, per `expansion.md`) and collects every `smelt.<path>` reference whose resolved target declares a `timeseries:` block. The result must be exactly one — the **driving source**, whose `partition_column` and `granularity` parameterise the per-window step loop, the source-filter pushdown, and the run-window clamp.

| Cardinality of timeseries-tagged sources | Outcome |
|---|---|
| 0 | Rejected: `AccumulatingSnapshotNoDrivingSource`. |
| 1 | Accepted. |
| ≥ 2 | Rejected: `AccumulatingSnapshotMultipleDrivingSources`. |

Non-timeseries sources in the FROM clause (lookups) are allowed and read in full on every window step. A *fact-to-dimension join* that brings the enriching event in as a separate joined relation is **not** this shape — see §"Classifier checks" (join-expressed enrichment) and Design §"Model it as a keyed union".

### The attribution horizon

The horizon `H` is the maximum time by which an enriching fact may arrive *after* the event it enriches. It bounds the run-window clamp: a run over `[run_start, run_end)` may only touch keys whose event time is `≥ run_start − H`. `H` is a **watermark-style completeness bound** (`docs/research/20260703-model-updates.md` §8.6): an enriching fact arriving more than `H` after its event is **dropped** (the key is considered settled), unless a full refresh is run.

`H` must be **bounded**. It is resolved in one of two ways:

1. **Derived** — when the model expresses the attribution window as a forward predicate on the driving source, e.g. `conversion_ts BETWEEN event_ts AND event_ts + INTERVAL '30 days'`. The `+ INTERVAL '30 days'` is a Form-B forward reach; `H = 30 days` is read from the SQL (derive-don't-declare). This is the preferred form.
2. **Declared** — on the *source*, as a source-lateness property shared by every consumer (the `timeseries:` source declaration, default `0`). Used when the horizon is a pipeline property not expressible as a SQL predicate.

An **unbounded** horizon — no forward predicate and no source-lateness declaration that bounds it — is rejected (`AccumulatingSnapshotUnboundedHorizon`): with `H → ∞` the clamp `run_start − H → −∞`, every run would touch all history and retain unbounded hot state. This is the exact mirror of `batched` rejecting an `UNBOUNDED PRECEDING` lookback frame (`batched_models.md` §"Batch safety classification").

There is **no per-model horizon override**. `H` is either read from the model's own SQL (derived) or inherited from the driving source's declaration (a world-fact shared by every consumer of that source). A per-model override that diverged from the source's declared lateness would let one consumer claim a completeness bound the source does not honour; the two resolution paths above are the whole surface.

### Classifier checks

A `refresh: accumulating_snapshot` model is rejected at planning time if any of these hold on the inlined outer SELECT (after function expansion):

1. **No `GROUP BY`** — `AccumulatingSnapshotRequiresGroupBy`.
2. **Non-key projection is not an allowlisted milestone aggregator** — `AccumulatingSnapshotUnknownCombiner`. Composite expressions over aggregates are rejected; add columns for the underlying milestones and derive downstream.
3. **A milestone is not provably once-write** — `AccumulatingSnapshotCorrectableMilestone`. This check applies **only to `COALESCE`-first-non-null milestones**: `MIN`/`MAX`/`MIN_BY`/`MAX_BY` are order-independent monoids whose merge converges regardless of once-write, so they need no proof (Surface §"Milestone combiner allowlist"). A `COALESCE`-first-non-null milestone is once-write — at most one distinct non-null value per key — and therefore admitted, only when the classifier can prove it via one of a **bounded set of provable forms**:
   - **Key-derived** — the `COALESCE` argument is a function of the `GROUP BY` key alone (trivially constant per key).
   - **Source-declared functional dependency** — the driving source declares `key → column` (the value is a per-key constant by the source's own contract).

   Any `COALESCE`-first-non-null milestone the classifier cannot place in one of these forms **fails closed** — the once-write provenance analysis is deliberately conservative, refusing rather than assuming a value is per-key constant. (`MIN`/`MAX`/`MIN_BY`/`MAX_BY` are never rejected by this check.)
4. **Enrichment expressed as a fact-to-dimension join** — `AccumulatingSnapshotJoinExpressedEnrichment`. A join between an events relation and a separately-arriving conversions relation is the bilinear operator smelt cannot self-maintain (Design §"Model it as a keyed union"); the model must union both into one keyed driving stream.
5. **`GROUP BY` contains the driving source's `partition_column`** — `AccumulatingSnapshotGroupByContainsPartitionColumn` (that is the batched shape).
6. **Non-deterministic functions in the outer body** — `AccumulatingSnapshotForbidsNondeterministic`.
7. **Unbounded forward horizon** — `AccumulatingSnapshotUnboundedHorizon` (§"The attribution horizon").

There is **no** `safety_overrides:` block. The rejected constructs break the end-state equivalence contract (correctable milestones need retraction; join enrichment needs general IVM), not merely a partial-correctness property — there is no bypass (Design §"No safety_overrides").

### Once-write end-state equivalence

For any set of driving-source windows `S = {W₁, …, Wₙ}` and any ordering π over `S`:

```
accumulating_snapshot_run(model, π(S))  ==  full_refresh(model, source.where(partition_col ∈ S))
```

The stored value of each milestone column for each key equals what a full refresh would compute over the same set of source windows, independent of the order (or overlap) in which they were processed. This holds because every milestone combiner is a **commutative, associative, idempotent monoid** with identity `NULL`, and once-write guarantees at most one distinct non-null value per key — so `LEAST`/`GREATEST`/`COALESCE` over any ordering, with repeats, converge to the same value.

This contract is the maintained-relation equivalence of `cumulative_aggregate.md` §"Cross-partition equivalence", specialised **per milestone column** to the once-write case, and it is **structurally different** from batched's per-partition equivalence — there is no `partition_column` to slice by; the promise is per-key end state.

### The maintenance boundary (algebraic ladder)

On the ladder of `cumulative_aggregate.md` §"The maintenance boundary" (from `docs/research/20260703-model-updates.md` Part 14), the milestone combiners sit on the first rung and no higher:

- **Direct monoid — the whole of this mode.** `LEAST`/`GREATEST`/`COALESCE`/`MAX_BY` are commutative monoids (identity `NULL`/±∞), directly presentable, `merge_into`-maintainable on a plain engine with no native IVM. Because they are additionally **idempotent**, re-merging an already-applied window is a no-op (§"Overlap tolerance") — a stronger property than `cumulative`'s non-idempotent `SUM`/`COUNT`.
- **Not a group.** These combiners are monoids but **not** groups: you cannot un-see a set milestone without the underlying multiset. A milestone that can be *revised* (retracted and replaced) is therefore out of scope and fails closed (`AccumulatingSnapshotCorrectableMilestone`). Retraction/correction is the group rung, delegated to engine-native IVM via `refresh: materialized_view`.

### Windowed consumption

Batch-by-batch consumption is **not** a property of this mode — it is the input-consumption axis (`docs/research/20260703-model-updates.md` §19), orthogonal to the refresh mode and **derived** from the driving source's shape:

- Because the driving source carries a `timeseries:` clock, the model is consumed **window-forward**: the `--event-time` run window applies to the *source's* `partition_column`, only the new tail is read, and the run steps over covered windows in temporal order (exactly as `cumulative` consumes its driving source).
- The **clamp** falls out of the horizon: a window `[run_start, run_end)` merges into keys with event time `≥ run_start − H`. State kept *hot* (mergeable) is only keys within `H` of the current window; older keys are settled.

Windowed consumption is not selected by any knob — there is no `strategy:` selector and no window field on the model (Design §"One declaration").

### Overlap tolerance

Because the milestone combiners are idempotent monoids, run windows may be processed **out of order, backfilled in slices, or run in parallel**, and any window may be **re-run**, without corrupting state (`docs/research/20260703-model-updates.md` §19.4). Re-merging an already-applied window converges to the same value (`GREATEST(x, GREATEST(x, y)) = GREATEST(x, y)`). The mode therefore needs **no** precise DELETE-covers-INSERT write-window invariant — there is no DELETE, and the clamp is a *work* bound (which keys are eligible to be touched), not a *correctness* bound.

### The hot-key set and its space cap

The keys eligible to be merged in a given run — those with event time `≥ run_start − H` (the clamp, §"The attribution horizon") — are the **hot set**. Keys older than that are **settled**: no future in-window delta can reach them.

This mode does **not** garbage-collect settled keys from the stored table — the output is a full keyed lookup and every key, hot or settled, remains readable. What is bounded is the *work*: only hot keys are candidates for a `merge_into` on any run. The stored table therefore grows with the total key space, exactly as a full-refresh table would; the clamp bounds only how far back a run reaches, not the table size.

To keep the mode fail-loud (`architecture.md` §"Fail-loud discipline"), the rule asserts a **cap on the number of keys touched in a single run's merge** (the per-run hot-key working set). If a run's delta would merge more keys than the cap, the rule **errors** — it does not silently process an unbounded working set — and the diagnostic steers the operator to narrow the run window or run a full refresh. The cap is a coarse guard against a mis-derived or unbounded-in-practice horizon, not a correctness mechanism; the concrete default and whether it is operator-tunable are settled at implementation time (§Known Divergences).

### Source-filter pushdown

For the **driving source**, the rule injects a per-window `WHERE <partition_column> >= W AND < W + granularity` on the source reference, once per window step. For **non-timeseries sources** (lookups), no pushdown happens — they are read in full each window, mirroring `batched_models.md` §"Source-filter pushdown".

### Functions inside bodies

Function expansion (`expansion.md`) runs **before** the classifier. Milestone-projection reading, GROUP-BY inspection, FROM-clause walking, horizon derivation, and pushdown operate on the expanded CST. A `smelt.define`-resolved milestone aggregator is admitted only if its expanded body produces an allowlisted once-write combiner at the outermost expression position; opaque calls (`smelt.extern`, non-inlinable built-ins) are rejected via `AccumulatingSnapshotUnknownCombiner`.

### Interaction with `--auto` / staleness

`--auto`'s staleness analysis for an accumulating-snapshot model re-processes the stale driving-source windows: any window whose input changed is re-stepped, and because the combiners are idempotent, re-running it is always safe (unlike `cumulative`'s reversible-aggregator caveat) — no widening of the read window is required, since `merge_into` only touches the keys present in each re-run window's deltas. The precise staleness *fidelity* — whether `--auto` re-runs exactly the changed windows or conservatively re-runs from the earliest stale point — is tied to the eviction/settled-key decision (§Known Divergences).

### `unique_key` and column naming

The rule derives `unique_key` from `GROUP BY`. Output column names are the projection list's `AS` aliases (or source column names). `MAX(conversion_ts) AS converted_at` produces a `converted_at` column holding the latest observed conversion time per key across all merged windows.

## Design

**Keyed MERGE, not DELETE+INSERT.** Retroactive enrichment updates a *small fraction* of past rows (only keys that gained a milestone this window) and may reach *far* back (an event converts many days later). Batched's whole-partition DELETE+INSERT would rebuild every row of a touched partition to update a handful — the wrong write primitive. This mode uses the keyed `merge_into` primitive: touch only the keys in the delta, combine per column. It is the maintained camp's write path (`docs/research/20260703-model-updates.md` §13.2), not batched's. *Modelling enrichment as a batched model with a widened outer clamp* was rejected because the reach is sparse and potentially long — batched rebuild cost scales with partition size, not with the number of changed rows.

**Derive the combiner from the SQL.** `GROUP BY` names the key; each milestone projection names its aggregator; the cross-window combiner is a fixed lookup (`MIN → LEAST`, `MAX → GREATEST`, `COALESCE → COALESCE`). *A `milestones:` config block listing columns and combiners* was rejected for the same reason `cumulative` rejects an `aggregators:` block: it re-introduces the metadata-vs-SQL drift the maintained family exists to avoid (`docs/research/20260521-incremental-as-planner-rule.md`). If it is in the SQL, it is not also in YAML.

**Bounded horizon, or refuse.** The clamp — and therefore batch-by-batch consumption and bounded hot state — exists only if the forward reach `H` is finite. An unbounded horizon collapses the clamp and forces full-history scans with unbounded state. Rather than silently degrade, the classifier refuses (`AccumulatingSnapshotUnboundedHorizon`), the mirror of batched refusing `UNBOUNDED PRECEDING`. `H` is derived from a `BETWEEN … + INTERVAL` forward predicate where present (the `after_secs` mirror of the batched lookback walk) and declared on the source otherwise — the derive-where-you-can / declare-where-you-must posture of `docs/research/20260703-model-updates.md` §8.6, which cleanly separates *computation reach* (derivable) from *source lateness* (a world-fact that must be declared).

**Once-write only; correction is a different mode.** The milestone combiners are monoids, not groups (`docs/research/20260703-model-updates.md` §14.2) — they cannot retract a value. This mode therefore admits only columns that fill once (NULL → set) and refuses revisable milestones. *Admitting correctable milestones with a subtract-then-add scheme* was rejected: it requires stored, invertible per-key deltas (the group rung) and is properly the territory of engine-native IVM via `refresh: materialized_view`. Keeping the mode once-write is what lets it run smelt-driven on a plain engine.

**Model it as a keyed union, not a join.** The enriching fact (a conversion) and the enriched event share a key and must both flow through the *one* driving stream, so the milestone combiner is a per-key monoid merge. Expressed instead as `events ⋈ conversions`, enrichment becomes a bilinear join — the general-operator slice smelt cannot self-maintain (`docs/research/20260703-model-updates.md` §14.3), routed to `refresh: materialized_view` or DAG composition. The classifier refuses the join form (`AccumulatingSnapshotJoinExpressedEnrichment`) and points at the union rewrite.

**One declaration, everything else derived.** Following the vertical-declared / horizontal-derived split (`docs/research/20260703-model-updates.md` §19.3): the *contract* (`refresh: accumulating_snapshot`) is declared; the *scan* (window-forward, batch-by-batch, clamped) is derived from the driving source carrying a `timeseries:` clock, exactly as for `cumulative`. *A `strategy:` selector or a `batched_snapshot:` mode* was rejected as the dbt-`incremental_strategy` anti-pattern the maintained family is designed against — a knob that silently changes the contract.

**No `safety_overrides`.** Batched exposes `safety_overrides:` because some of its rejections guard a *partial-correctness* optimisation the modeller may knowingly waive. This mode's rejections instead guard the *equivalence contract itself* — a correctable milestone genuinely needs retraction, a join genuinely needs general IVM — so there is nothing safe to waive. The escape from a rejection is to remodel (union form, bounded predicate) or move to `refresh: materialized_view`, not to override.

**Output is not a timeseries.** One row per key, no partition column; the model forbids `timeseries:` on itself and reads its partition shape from the driving source. Downstream consumers treat it as a lookup. This is the same boundary `cumulative`, `latest_value`, and `versioned` draw.

**One windowed executor, shared with `cumulative`.** The window-forward step loop — classify → step over the driving source's partitions in temporal order → per-partition source-filter pushdown → create-or-`merge_into` — is *identical* across the smelt-maintained keyed modes; only the classifier and the per-column combiner differ. Accumulating snapshot therefore does **not** copy `cumulative`'s executor: the loop is factored into a single **windowed-keyed-maintenance driver** parameterised by `(classifier, merge-SQL builder)`, and both modes (and, prospectively, `latest_value` / `versioned`) consume it. *A per-rule copy of the step loop* was rejected as the drift risk `docs/research/20260703-model-updates.md` §19.8/§20.9 flags — four near-identical clamp/inject/merge loops would diverge. A consequence: the mode inherits the shared driver's granularity support (`cumulative`'s `day`/`week` today); widening granularities is a property of the shared driver, not of this mode.

## Constraints & Invariants

1. **Opt-in is `refresh: accumulating_snapshot` alone** (storage implied `table`). No rule-specific config block.
2. **`timeseries:` and a `batched:` block are forbidden on the model.** Diagnostics `AccumulatingSnapshotForbidsTimeseries`, `AccumulatingSnapshotForbidsBatched`.
3. **`unique_key` is derived from `GROUP BY`.** No `GROUP BY` → `AccumulatingSnapshotRequiresGroupBy`.
4. **Milestone combiners are a fixed lookup off the projection aggregator.** Authors do not declare combiners.
5. **Every milestone combiner is a commutative, associative, idempotent monoid.** Non-monoid or correctable (revisable) milestones are refused.
6. **Every milestone is once-write** (NULL → set, never set → different-set). Correction is out of scope.
7. **The forward-attribution horizon `H` is bounded** — derived from a forward predicate or declared on the source. Unbounded → refused.
8. **The run-window clamp is `run_start − H`** on the driving source's clock; it never depends on data values (it is a function of the run window and `H` only).
9. **Enrichment is a keyed union over one driving source, not a fact-to-dimension join.**
10. **End-state equivalence holds for any ordering and any overlap** of processed windows (idempotent-monoid merge).
11. **No `partition_column` on the output.** Downstream treats it as a lookup.
12. **No silent downgrade.** A classifier rejection refuses the model at planning time — no fallback to batched, to full-refresh, or to `materialized_view`.
13. **The per-run hot-key working set is capped and fail-loud.** A run whose delta would merge more keys than the cap errors rather than processing an unbounded working set (Semantics §"The hot-key set and its space cap"). Settled keys are never GC'd from the stored table.
14. **The windowed step loop is shared, not copied.** Accumulating snapshot consumes the same windowed-keyed-maintenance driver as `cumulative` (Design §"One windowed executor").
15. **`COALESCE`-first-non-null milestones require a once-write provenance proof; `MIN`/`MAX`/`MIN_BY`/`MAX_BY` do not.** Unprovable `COALESCE` milestones fail closed.
16. **`H` has no per-model override** — derived from the model SQL or declared on the driving source, nothing else.

## Known Divergences / Open Questions

- **Not implemented.** Declaring `refresh: accumulating_snapshot` currently produces an unknown-refresh-value error. The classifier, the milestone-combiner derivation, and the windowed `merge_into` execution are unbuilt. The design is worked in `docs/research/20260703-model-updates.md` Part 20; the delivery plan is `docs/plans/20260704-accumulating-snapshot.md`.
- **The forward-reach walk `H` consumes now exists (`source_bounds.rs`); the accumulating-snapshot classifier that reads it is still unbuilt.** A derived horizon from a `BETWEEN … + INTERVAL` forward predicate (or a bounded `RANGE … FOLLOWING` frame) reads the `after_secs` half of the source bound — the mirror of the batched lookback (`before_secs`) walk, landed by `docs/plans/20260704-model-updates-group-b.md` (phase B2). The classifier, combiner derivation, shared executor, and windowed merge that would consume it are unbuilt (see "Not implemented" above); wiring the derived-`H` path into this mode's own classifier is scope for `docs/plans/20260704-accumulating-snapshot.md`, not gated on any further Group B work.
- **The hot-key cap default is unspecified.** The mode caps the per-run hot-key working set and fails loud when exceeded (Semantics §"The hot-key set and its space cap"); the concrete default cap and whether it is operator-tunable are settled at implementation time. This is a value, not a design fork.
- **Settled-key GC is a deferred enhancement, not v1.** v1 keeps every key (hot and settled) in the stored lookup and never garbage-collects — the table grows with the total key space, as a full-refresh table would. A future §14.4-style space-budget GC that retires keys older than `run_window − H` from a *hot-state store* is possible if a persistent-watermark store is added (`docs/research/20260703-model-updates.md` §20.9), but v1 needs none: the clamp already bounds the per-run work, and the fail-loud cap guards the working set.
- **`COALESCE` once-write provenance breadth may widen.** v1 admits a `COALESCE`-first-non-null milestone only via the two provable forms (key-derived; source-declared functional dependency — Semantics §"Classifier checks"), failing closed otherwise. Broadening the provable set (e.g. tracing a value through CTEs/subqueries to establish per-key constancy) is a future refinement; the conservative prover holds until then. `MIN`/`MAX`/`MIN_BY`/`MAX_BY` are unaffected — they need no proof.
- **Non-determinism run-pinning is a deferred alignment.** v1 rejects `NOW()`/`RANDOM()` outright. Adopting `batched`'s compile-time run-pinning of `NOW`/`CURRENT_*` (once that lands and is proven in `batched` — `docs/plans/20260704-model-updates-group-b.md` B3) is a later step; the conservative reject holds until then.
- **Sibling keyed modes.** `cumulative` (running aggregate), `latest_value` (Type 1), and `versioned` (Type 2) are peers on the refresh axis with their own specs and classifiers, not variants of this rule. They share the windowed-keyed-maintenance driver this mode also consumes (Design §"One windowed executor"). The engine-maintained counterpart of any keyed mode is `refresh: materialized_view`, not a maintainer flag here.

## References

- **Code**:
  - `crates/smelt-core/src/config.rs` — the `refresh` axis enum (host for the `accumulating_snapshot` value)
  - `crates/smelt-core/src/metadata.rs` — frontmatter extraction; validation that `timeseries:` / `batched:` are absent when `refresh: accumulating_snapshot`
  - `crates/smelt-logical/src/rules/` — host for the accumulating-snapshot classifier (pure rule-data in `smelt-logical`; see `architecture.md` §"Layered single-ownership")
  - `crates/smelt-logical/src/analysis/source_bounds.rs` — the forward-reach (`after_secs`) derivation the horizon consumes (mirror of the lookback walk)
  - `crates/smelt-runtime/src/cumulative.rs` (→ to be generalised into a shared `windowed-keyed-maintenance` driver) — the window-forward step loop this mode shares with `cumulative` (Design §"One windowed executor")
  - `crates/smelt-backend/src/lib.rs` — `merge_into` trait method (physical primitive the rule calls, shared with `cumulative`)
  - `crates/smelt-backend-duckdb/src/lib.rs` — DuckDB `merge_into` implementation
- **Tests**:
  - Accumulating-snapshot classifier unit tests and once-write / end-state equivalence tests (to be added alongside the implementation plan)
- **User docs**: `docs-site/docs/guide/materializations.md` (to document `refresh: accumulating_snapshot` on the refresh axis, alongside `batched` and `cumulative`)
- **Plans (history)**:
  - `docs/plans/20260704-accumulating-snapshot.md` — this mode's delivery plan (classifier, shared windowed executor, combiner + once-write prover, horizon, hot-key cap)
  - `docs/plans/20260704-model-updates-group-b.md` — the batched-eligibility work whose B2 phase lands the shared forward-reach (`after_secs`) walk this mode's derived horizon consumes
- **Research**:
  - `docs/research/20260703-model-updates.md` — Part 20 (this mode's worked design); §13 (maintained camp), §14.1–14.2 (monoid/group ladder), §19.1–19.4 (input-consumption axis, windowed keyed consumption), §8.3/§8.6 (forward reach, the `FOLLOWING` mirror, watermark completeness bound)
  - `docs/research/20260522-cumulative-as-its-own-rule.md` — §"Sibling rules beyond cumulative_aggregate" (the earlier accumulating-snapshot sketch)
  - `docs/research/20260521-incremental-as-planner-rule.md` — the "derive from SQL, not YAML" principle this spec inherits
- **Related specs**:
  - `cumulative_aggregate.md` — the running-aggregate keyed peer; structural template and shared `merge_into` execution / windowed-consumption axis
  - `latest_value_models.md`, `versioned_models.md` — the other smelt-maintained keyed modes
  - `materialized_view.md` — engine-native IVM; where correctable milestones and join-expressed enrichment are delegated
  - `batched_models.md` — the partitioned-output peer whose lookback clamp this mode mirrors on the driver axis
  - `timeseries.md` — the source-side declaration (partition column, granularity, source-lateness) this rule consumes
  - `models.md` — the three axes (kind / storage / refresh); refresh-axis host; frontmatter table
  - `expansion.md` — function expansion; runs before the classifier
  - `architecture.md` — `smelt.<path>` addressing; backend primitive contract; layered single-ownership
