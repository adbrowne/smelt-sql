---
feature: cumulative_aggregate
status: experimental
last_reviewed: 2026-07-04
owners: [andrew]
---

# Cumulative Aggregate Refresh Mode

> **What this is.** A normative spec for the `refresh: cumulative` mode — a stateful-merge planner rule that collapses a timeseries source into one row per key, where each row reflects state across all processed source partitions. Cumulative is a **keyed-output** value of the **refresh axis** (`models.md` §"Refresh axis") on a stored `table`: the stateful counterpart of the partitioned-output `batched` mode, and one of the keyed modes alongside `versioned`, `latest_value`, and `materialized_view`. Covers the frontmatter selector, the classifier, the per-partition delta-SELECT shape, the cross-partition combine semantics, and the rules around what may be expressed. The **processed-input equivalence invariant** (its end-state specialisation) and the **algebraic maintenance ladder** that govern this mode are owned by `model_maintenance.md`; this spec is their **reference implementation** for the keyed-maintenance path and cites them rather than redefining them. Out of scope: batched DELETE+INSERT (`batched_models.md`), the `timeseries:` declaration this rule consumes from its source (`timeseries.md`), full model frontmatter schema (`models.md`), the backend `merge_into` primitive (described in `architecture.md` §"Backend primitives" — the cumulative rule is one caller).
>
> **Spec-first rule.** Edit this file before writing the implementation plan. The spec diff is the change description.
>
> **Timeless-oracle rule.** This spec describes the feature as if it has always existed. Implementation status lives in §Known Divergences (with plan link) or §References → Plans (history).

## Surface

### YAML frontmatter (in `.sql` files)

```sql
---
refresh: cumulative
---

SELECT
    device_id,
    user_id,
    COUNT(*)      AS event_count,
    MIN(event_ts) AS first_seen,
    MAX(event_ts) AS last_seen
FROM smelt.silver.events_parsed
WHERE user_id IS NOT NULL
GROUP BY device_id, user_id
```

`refresh: cumulative` is the entire opt-in; it implies a stored `table` (`models.md` §Design — the modeller does not restate `materialization: table`). No other frontmatter key is read or required by the rule.

`refresh: cumulative` **forbids** a `timeseries:` block on the model — the output has no partition column (Semantics §"Output shape"). It **forbids** a `batched:` block — cumulative and `batched` are different refresh modes with different equivalence contracts (`batched_models.md`).

### `smelt.yml` (project-level overrides)

```yaml
models:
  device_user_edges:
    refresh: cumulative
```

Frontmatter wins over `smelt.yml` when both set `refresh`. The same forbid-`timeseries:` / forbid-`batched:` constraints apply.

### CLI

A `refresh: cumulative` model consumes the same `--event-time-start`/`--event-time-end` flags as batched execution — the run window names the source partitions that will be merged in. Format and alignment rules follow `batched_models.md` §"CLI". The flags apply to the driving source's `partition_column` / `granularity` (Semantics §"Driving source"), not to any column on the cumulative output.

```
smelt run --event-time-start <ISO-8601> --event-time-end <ISO-8601> [selectors]
smelt backbuild --event-time-start <ISO-8601> --event-time-end <ISO-8601> [selectors]
```

### Aggregator allowlist

The classifier accepts non-key projections that are direct calls to one of:

| Per-partition aggregator | Cross-partition combiner |
|---|---|
| `COUNT(...)` | `SUM` |
| `SUM(...)` | `SUM` |
| `MIN(...)` | `MIN` |
| `MAX(...)` | `MAX` |
| `BOOL_AND(...)` | `BOOL_AND` |
| `BOOL_OR(...)` | `BOOL_OR` |
| `BIT_AND(...)` | `BIT_AND` |
| `BIT_OR(...)` | `BIT_OR` |
| `BIT_XOR(...)` | `BIT_XOR` |

Any other aggregate function, or any non-aggregate non-key expression, in the projection list is rejected by the classifier (Semantics §"Classifier checks"). The combiner column is a fixed lookup off the per-partition aggregator name; authors do not declare combiners.

### Diagnostic codes (owned by this spec)

| Code | Severity | Trigger |
|---|---|---|
| `CumulativeRequiresGroupBy` | Error | The model SELECT has no `GROUP BY` — there is no unique key to derive. |
| `CumulativeForbidsTimeseries` | Error | The model declares both `refresh: cumulative` and a `timeseries:` block. |
| `CumulativeForbidsBatched` | Error | The model declares both `refresh: cumulative` and a `batched:` block. |
| `CumulativeUnknownAggregator` | Error | A non-key projection is not a direct call to an aggregator in the allowlist. The diagnostic names the offending aggregator and points at the projection. |
| `CumulativeGroupByContainsPartitionColumn` | Error | The `GROUP BY` list contains the driving source's `partition_column`. The diagnostic suggests switching to `refresh: batched` + `timeseries:` instead. |
| `CumulativeForbidsWindowFunctions` | Error | The outer SELECT body uses `OVER (...)`. The cumulative state *is* the window; window functions in cumulative SQL are nonsensical. |
| `CumulativeNoDrivingSource` | Error | No `smelt.<path>` reference in the FROM clause has a `timeseries:` declaration on the resolved target. |
| `CumulativeMultipleDrivingSources` | Error | More than one timeseries-tagged source appears in the FROM clause. The diagnostic lists the candidate sources. |
| `CumulativeForbidsNondeterministic` | Error | The SQL uses `NOW()`, `RANDOM()`, or other non-deterministic functions outside stable contexts. Cross-partition combine requires deterministic per-partition output. |

## Composition

Per `model_maintenance.md` §"The composition contract", cumulative is composed from capabilities owned by the framework specs; this table states what it draws from each. The mode's own local machinery (classifier, allowlist, reprocessing, driving-source resolution, `unique_key` derivation) is defined in full in §Semantics below.

| Composition slot | What cumulative uses | Owner |
|---|---|---|
| **Properties required** | algebraic discriminants (is-monoid / needs-inverse / decomposable / value-vs-order-monotone — the combiner algebra); driving-fact / anchor resolution (pick the single timeseries-tagged source); event-time monotonicity trace (the driving source's clock is monotone) | `model_properties.md` |
| **World-facts consumed** | the **timeseries clock** (`partition_column` / `granularity`) of the driving source; the **source mutation profile** (append-only vs mutable — gates whether reprocessing is even reachable) | `timeseries.md`, `sources.md` |
| **Transforms driven** | keyed `merge_into` via the **windowed-keyed-maintenance driver** + **source-filter pushdown**; for the higher rungs, **hidden decomposed state + presentation view** (rung 2), **retraction via delta history** (rung 3), and **explicit bounded-domain multiset** (rung 4) | `model_transforms.md` |
| **Output shape** | **keyed** — one row per `unique_key`, no `partition_column` | `models.md` §"Refresh axis" |

The correctness contract (end-state equivalence) and the ladder that orders these transforms are owned by `model_maintenance.md` (§"The equivalence invariant", §"The algebraic maintenance ladder"); cumulative validates its declared mode against the derived properties and refuses fail-loud — it never chooses or downgrades the mode (`model_maintenance.md` §"Validator, not chooser").

## Semantics

### Execution model

For a `refresh: cumulative` model with a run window `[run_start, run_end)`:

1. **Classify the model's SQL** (§"Classifier checks") and derive:
   - `unique_key` — the columns named in `GROUP BY`.
   - `aggregator_columns` — a map from each non-key projection's output column name to its `(per_partition_agg, cross_partition_combiner)` pair, looked up from the Surface §"Aggregator allowlist" table.
   - `driving_source` — the single timeseries-tagged source in the FROM clause (Semantics §"Driving source").
2. **Step over source partitions** in temporal order. For each source partition `D` covered by the run window:
   - **Source-filter pushdown** injects `<driving_source>.<partition_column> >= D AND <driving_source>.<partition_column> < D + granularity` onto the driving source's reference. Sources without `timeseries:` are not pushdown candidates; they are read in full each partition.
   - **Execute the per-partition delta SELECT** through the engine, producing one delta row per `unique_key` value present in this partition's input.
   - **Backend `merge_into` call** with the derived `unique_key` and a per-column combiner map. Matched rows: target's value is combined with delta's value via the cross-partition combiner (`target.event_count + delta.event_count`, `LEAST(target.first_seen, delta.first_seen)`, `GREATEST(target.last_seen, delta.last_seen)`, etc.). Unmatched rows: insert as-is.
3. If the output table does not exist when the first partition is merged, the rule creates it from the first partition's delta SELECT (`CREATE TABLE AS SELECT`); subsequent partitions are merged into it.

A run window covering N source partitions performs N `merge_into` calls in temporal order. Each `merge_into` is one backend transaction. Earlier committed partitions do not roll back on a later failure — partial progress is intentional. Re-running the same `[run_start, run_end)` over unchanged source data converges to the same final state **only for idempotent combiners** (`MIN`, `MAX`, `BOOL_AND`, `BOOL_OR`, `BIT_AND`, `BIT_OR` — where re-applying an already-merged partition's delta is a no-op). `SUM`, `COUNT`, and `BIT_XOR` are **not** idempotent: re-merging an already-applied partition double-counts, so retry-after-partial-failure for these aggregators relies on the reprocessing machinery in Semantics §"Reprocessing semantics", not a blind re-run.

### Output shape

A cumulative aggregate model's output has:

- One row per `unique_key` value (where `unique_key` is the `GROUP BY` column list).
- Per-key columns whose values reflect the cross-partition combine of every processed source partition's contribution.
- **No** `partition_column`. **No** `event_time_column`. **No** `timeseries:` declaration on the model itself.

Downstream consumers see the cumulative output as a lookup table — there is no partition information to push down. Downstream models join to it and read it in full each run, identical to the treatment of any non-timeseries source (`batched_models.md` §"Source-filter pushdown").

### Driving source

The single driving input is resolved by the shared **driving-fact / anchor resolution** proof (`model_properties.md`): among the inlined outer SELECT's FROM references (after function expansion, per `expansion.md`), exactly one must be the anchor — for cumulative, the one `smelt.<path>` whose resolved target declares a `timeseries:` block. The proof's exactly-one verdict maps to cumulative's diagnostics:

| Driving-source cardinality | Outcome |
|---|---|
| 0 | Rejected: `CumulativeNoDrivingSource`. The error message suggests declaring `timeseries:` on the source or switching the materialization. |
| 1 | Accepted. The driving source's `partition_column` and `granularity` parameterise the per-partition step loop and the source-filter pushdown. |
| ≥ 2 | Rejected: `CumulativeMultipleDrivingSources`. A future plan may add explicit `driven_by:` disambiguation for same-granularity sources (Known Divergences). |

The driving source's `granularity` must be `day` or `week`. Any other granularity — `hour`, `month`, `quarter`, or `year` — is rejected at runtime by the per-partition step loop (see Known Divergences). Non-timeseries sources in the FROM clause (lookups) are allowed and are read in full on every partition step. (The current implementation resolves the driving source with a mode-local ref-count over `timeseries:`-tagged refs rather than the shared alias-scoped proof; consolidating the two onto one resolver is tracked in `docs/plans/20260704-model-updates.md` — see Known Divergences.)

### Classifier checks

A `refresh: cumulative` model is rejected at planning time if any of these hold on the inlined outer SELECT (after function expansion):

1. **No `GROUP BY` clause** — `CumulativeRequiresGroupBy`.
2. **Non-key projection is not an allowlisted aggregator call** — `CumulativeUnknownAggregator`. Each projection that is not in the `GROUP BY` must be a direct call to one of the Surface §"Aggregator allowlist" functions, optionally with `AS <output_name>`. Composite expressions over aggregates (`SUM(x) + 1`, `MIN(x) / MAX(y)`) are rejected; authors must add columns for the underlying aggregates and compute derived values downstream.
3. **`GROUP BY` contains the driving source's `partition_column`** — `CumulativeGroupByContainsPartitionColumn`. Including the partition column in the key produces the per-partition shape, not the cumulative shape; the diagnostic suggests switching to `refresh: batched` + `timeseries:`.
4. **Window functions in the outer body** — `CumulativeForbidsWindowFunctions`. Any `OVER (...)` clause on a projection in the outermost SELECT.
5. **Non-deterministic functions in the outer body** — `CumulativeForbidsNondeterministic`. `NOW()`, `CURRENT_TIMESTAMP`, `RANDOM()`, etc.

Additionally, the `Surface §"Diagnostic codes"` rejections for `CumulativeForbidsTimeseries`, `CumulativeForbidsBatched`, `CumulativeNoDrivingSource`, and `CumulativeMultipleDrivingSources` fire at workspace load or planning time as named.

There is no `safety_overrides:` block for the cumulative rule. The rejected constructs cannot be bypassed because they break the cross-partition equivalence contract, not just the per-partition equivalence contract — there is no partial-correctness escape hatch the way the `batched:` block has one for `allow_window_functions`.

### Cross-partition equivalence

Cumulative upholds the **end-state specialisation** of the processed-input equivalence invariant, defined once in `model_maintenance.md` §"The equivalence invariant": for any set `S = {D₁, …, Dₙ}` of processed source partitions and any ordering π over `S`, the maintained state equals `full_refresh(model, source.where(partition_col ∈ S))` — the result depends only on the *set* processed, not the order. This spec does not redefine the invariant; it is the load-bearing property the cumulative classifier upholds locally by admitting only commutative-and-associative combiners (Surface §"Aggregator allowlist") over a stable `GROUP BY` key, so reordering merges cannot change the final state. (Contrast batched's per-partition specialisation: cumulative has no `partition_column` to slice by, so it promises end-state equality, not per-slice equality.)

### The maintenance boundary

What a `refresh: cumulative` model can maintain is decided by the **algebra of its combiners**, laid out as the four-rung **algebraic maintenance ladder** owned by `model_maintenance.md` §"The algebraic maintenance ladder" (which also owns the rung ordering, the maintainable-vs-delegated cutoff, and the derivation). This spec does not restate the ladder; it records **where cumulative sits on it**:

- The Surface §"Aggregator allowlist" is exactly the closed set of **rung-1 direct commutative monoids** over scalar columns (`SUM`/`COUNT`, `MIN`/`MAX`, `BOOL_*`, `BIT_*`) — the whole of what the rule maintains today.
- The deferred **`AVG` rewrite** grows into **rung 2** (a decomposed monoid `(sum, count)` behind a presentation view — Known Divergences).
- **Reprocessing via delta history** for the reversible aggregators (`SUM`, `COUNT`, `BIT_XOR`) is **rung 3** (a commutative group; `MIN`/`MAX`/`BOOL_*`/`BIT_AND`/`BIT_OR` are monoids but **not** groups, which is why reprocessing them requires a full refresh — Semantics §"Reprocessing semantics").
- **Opt-in exact holistic aggregates** (`MEDIAN`/`PERCENTILE`/`MODE`, exact `COUNT(DISTINCT)`) grow into **rung 4** (an opt-in, fail-loud bounded-domain multiset). The opt-in is the model-scoped bounded-domain / space-budget declaration (`bounded_domain:`, `model_properties.md` §"Model-scoped declarations") — an explicit space-budget cap that unlocks the exact holistic rung only for the declared column; an absent cap is a configuration error, never a permissive default (Known Divergences).

Beyond the ladder — general-operator retraction over joins, non-additive state unbounded in a dimension the user cannot cap — is not smelt-driven-maintainable and is delegated to the engine's native incremental-view maintenance via `refresh: materialized_view` (`materialized_view.md`). The end-state equivalence contract (§"Cross-partition equivalence") holds unconditionally on every rung; only the state representation and its size change across rungs, never the fidelity of the user-visible value.

### Reprocessing semantics

If a source partition `D` has already been merged in and the source data at `D` changes, re-running the cumulative model over `[D, D + granularity)` does **not** produce a correct cumulative state, because the prior delta from `D` is already baked into the target table and a second merge adds it again.

The rule rejects reprocessing at planning time when it can detect it (the partition has been merged before and the run window includes it). The error message points at the two mitigations:

1. **Full refresh.** Re-run with `--full-refresh` (truncate-and-rebuild from the source). This is the v1-correct path.
2. **Cascade rebuild** — manual, no built-in support: truncate the target and re-run the cumulative model over every source partition from `D` onward.

Subtract-then-add (keeping per-partition deltas in a side table) is a candidate future shape for reversible aggregators (`SUM`, `COUNT`, `BIT_XOR`); see Known Divergences.

### Source-filter pushdown

Cumulative drives the shared **source-filter pushdown** transform (`model_transforms.md`) with a cumulative-specific parameterization: it is applied **per partition step**, not once per run. For the **driving source**, the rule injects a per-partition WHERE filter equivalent to:

```
WHERE <driving_source>.<partition_column> >= D
  AND <driving_source>.<partition_column> <  D + granularity
```

on the source reference in the inlined SELECT, where `D` ranges over the source partitions covered by the run window. The injection happens once per partition step, not once per run (this per-step application is cumulative's distinguishing use of the transform, vs batched's single run-window clamp).

For **non-driving timeseries sources** (forbidden by the v1 multiple-driving-source rule), no pushdown happens — but the configuration is rejected before pushdown runs.

For **non-timeseries sources** (lookups), no pushdown happens — they are read in full on each partition step. This mirrors `batched_models.md` §"Source-filter pushdown" — `timeseries:` on a source is the universal opt-in for being a pushdown target.

### Functions inside cumulative bodies

Function expansion (`expansion.md`) runs **before** the classifier and the driving-source walk. Classification, projection-list reading, GROUP-BY inspection, FROM-clause walking, and source-filter pushdown all operate on the expanded CST. A `smelt.define`-resolved aggregator call inside an outer-body projection is invisible to the classifier unless its expanded body produces an allowlisted aggregator at the outermost expression position; in that case it is admitted on the same terms as a hand-written allowlisted call.

Opaque calls (`smelt.extern` declarations, canonical built-ins that the analyser cannot inline) in the outer projection list are rejected — the classifier cannot prove they are allowlisted aggregators, so `CumulativeUnknownAggregator` fires.

### Interaction with `--auto` / staleness

`--auto`'s "what's stale" analysis for a cumulative model returns:

- **All reversible aggregators** (`SUM`, `COUNT`, `BIT_XOR`): "exactly the changed source partitions". A v1 implementation cannot honour this without delta-history bookkeeping; v1 returns "any partition ≥ the earliest stale partition" for safety. See Known Divergences.
- **Any irreversible aggregator** (`MIN`, `MAX`, `BIT_AND`, `BIT_OR`, `BOOL_AND`, `BOOL_OR`): "any partition ≥ the earliest stale partition; force a full refresh if earlier-than-current partitions are stale". Refusing reprocessing is the v1 policy (Semantics §"Reprocessing semantics").

### `unique_key` and column naming

The rule derives `unique_key` from the `GROUP BY` column list. The column names in the cumulative output are the projection list's `AS` aliases (or the source column names when no alias is given). Authors writing a cumulative model think about the cumulative output's column names directly: `COUNT(*) AS event_count` produces a column called `event_count` whose value reflects the cumulative count across all merged source partitions.

## Design

This section captures the load-bearing rationale.

**Cumulative is a separate refresh mode from batched, not a sub-knob of one.** dbt conflates the two under `materialized='incremental'` and dispatches by `incremental_strategy`. This is the single most common source of confusion in dbt because the `strategy:` knob silently changes the equivalence contract — same frontmatter, different invariants. smelt picks the opposite shape: each refresh mode is its own named value with its own contract. `batched` is per-partition-equivalent with a partitioned output; `refresh: cumulative` is cross-partition-equivalent with a per-key output. The two are different modes because they uphold different contracts on different output shapes — peers on the refresh axis (`models.md` §"Refresh axis"), not one nested under the other. Deeper rationale: `docs/research/20260522-cumulative-as-its-own-rule.md` §"Why per-partition equivalence is the wrong frame for cumulative".

**Derive `unique_key` and aggregators from the SQL, not from frontmatter.** The `GROUP BY` already names the key. Each non-key projection already names its per-partition aggregator. The cross-partition combiner is a fixed lookup table off the per-partition aggregator (`COUNT → SUM`, `MIN → MIN`, etc.). There is no information the rule needs that isn't already in the SELECT. *A `cumulative:` config block with `unique_key:` and `aggregators:` keys* was rejected because it re-introduces the metadata-vs-SQL drift problem the predecessor batched work explicitly removed (`docs/research/20260521-incremental-as-planner-rule.md`, "derive lookback from the SQL"). The same principle applies here: if a thing is in the SQL, do not also put it in YAML. The opt-in collapses to a single `refresh: cumulative` line (storage implied `table`), with no rule-specific config block.

**Cumulative output is not itself a timeseries.** The output has a unique key and aggregated columns, but no `partition_column` and no `granularity` — it has collapsed all source partitions into a single per-key row. The model therefore does not declare `timeseries:`; the rule reads the partition shape from the driving source's `timeseries:` declaration. Downstream consumers see the cumulative table as a lookup. *Allowing a `timeseries:` block on a `refresh: cumulative` model and letting it produce a per-partition output* was rejected because that shape is already what `batched` produces — there is no new behaviour, only ambiguity. The forbid-`timeseries:` rule (`CumulativeForbidsTimeseries`) makes the boundary structural.

**Fixed aggregator allowlist, not a registry.** v1 ships exactly the SQL aggregators that are provably commutative and associative under standard semantics (`COUNT`, `SUM`, `MIN`, `MAX`, `BIT_*`, `BOOL_*`). *Letting authors register custom combiners* was rejected for v1 because (a) the v1 web_analytics motivating example needs only the standard allowlist, (b) custom combiners would need a workspace-level registry surface that is out of scope here, and (c) extending the allowlist is additive and can be done without a spec change. `AVG`, `STRING_AGG`, `LIST_AGG`, `FIRST`, `LAST`, `APPROX_COUNT_DISTINCT` are intentionally out — `AVG` is rewritable to `SUM/COUNT` at read time but the rule does not perform that rewrite in v1.

**One driving source for v1, not multi-source.** Cumulative SQL with multiple timeseries-tagged sources is rejected (`CumulativeMultipleDrivingSources`). *Accepting multiple sources and looping over the finest granularity* was deferred because (a) it adds significant rule complexity (cross-source filter coordination) and (b) the motivating examples do not need it. An explicit `driven_by:` disambiguation field on the model is the natural follow-on, but adding it changes the "frontmatter is one line" property — it should land only when a concrete second-source motivator surfaces.

**Refuse reprocessing in v1, not subtract-then-add.** The simplest correctness contract is "merging a partition twice is undefined". Reprocessing requires either delta history (a side table per cumulative model) or a cascade rebuild from the changed partition forward. Both are more code than v1 should ship. *Silently double-counting reprocessed partitions* is the dbt-merge-strategy footgun and was rejected outright. The rule refuses to merge a partition that was already merged, with a diagnostic that points the author at `--full-refresh`.

**No `safety_overrides:` block.** The batched classifier offers per-check overrides (`allow_window_functions`, `allow_having`, etc.) because some rejected constructs only break *full-refresh equivalence*, and authors can knowingly accept partial-correctness. Cumulative's rejected constructs break the cross-partition equivalence contract itself — there is no partial-correctness fallback. A `safety_overrides:` knob would be a footgun: bypassing `CumulativeUnknownAggregator` for `STRING_AGG` would produce silently order-dependent output that is impossible to debug. The classifier is strict by design.

**`refresh: cumulative` lives on the refresh axis, not the `materialization` (storage) axis.** Cumulative does not add a variant to the storage enum (`View | Table | Ephemeral`); it is a refresh-axis value on an implied stored `table`, a peer of `batched` and the other keyed modes (`models.md` §"Refresh axis"). Modelling it as a storage value was rejected because it would put a refresh concern on the storage axis — the same conflation that motivated moving `materialized_view` off the storage axis (`models.md` §Design). The DuckDB `merge_into` backend primitive stays; it is the cumulative rule's physical strategy. The trait method, the implementation, and its tests do not move.

## Constraints & Invariants

1. **Opt-in is `refresh: cumulative` alone** (storage implied `table`). A cumulative model declares that one key and nothing else specific to this rule. There is no `cumulative:` configuration block.
2. **`timeseries:` and a `batched:` block are forbidden on cumulative models.** Diagnostics: `CumulativeForbidsTimeseries`, `CumulativeForbidsBatched`.
3. **`unique_key` is derived from `GROUP BY`.** A cumulative model without `GROUP BY` is rejected (`CumulativeRequiresGroupBy`).
4. **Per-column cross-partition combiner is a fixed lookup off the per-partition aggregator.** Authors do not declare combiners; the rule looks them up from the allowlist table.
5. **Allowlist is closed.** Aggregators outside the table are rejected. Composite expressions over aggregates are rejected. No `safety_overrides:` bypass.
6. **Exactly one driving source.** The classifier requires exactly one timeseries-tagged source in the inlined FROM clause.
7. **Cross-partition equivalence holds for any ordering.** Reordering merges across source partitions does not change the final cumulative state.
8. **No `partition_column` on the cumulative output.** Downstream consumers treat the cumulative table as a lookup.
9. **Reprocessing is refused in v1.** A run window that overlaps already-merged source partitions is rejected with a diagnostic; `--full-refresh` is the v1 mitigation.
10. **No silent downgrade.** A classifier rejection refuses the model at planning time. No fallback to full-refresh, no fallback to batched, no warning-then-continue.

## Known Divergences / Open Questions

- **Only the direct-monoid rung is implemented.** The algebraic ladder (`model_maintenance.md` §"The algebraic maintenance ladder") has four rungs; cumulative implements only rung 1 (direct commutative monoids — the current allowlist). Cumulative's placement on rungs 2–4 (decomposed monoid with a presentation view, group retraction, opt-in bounded-domain multiset) is recorded in Semantics §"The maintenance boundary" and specified ahead of implementation; each is delivered by a phase of `docs/plans/20260704-model-updates.md`. The three deferred features below — `AVG` rewrite, reprocessing via delta history, `--auto` staleness fidelity — are the same hidden-state mechanism (rungs 2–3) seen three times.
- **`AVG` rewrite (rung 2).** Out of scope today. The classifier refuses `AVG(...)`. A future phase stores `(sum, count)` state and presents `sum/count` through a presentation view (the decomposed-monoid rung), rather than a planning-time `SUM/COUNT` rewrite.
- **Multi-source disambiguation (`driven_by:`).** A cumulative model reading multiple timeseries-tagged sources is rejected in v1 (`CumulativeMultipleDrivingSources`). A future plan may add an explicit `driven_by: smelt.<source>` field on the frontmatter to pick among same-granularity candidates. Different-granularity sources are deferred indefinitely.
- **Self-referential cumulative.** A SELECT that joins to its own cumulative target (e.g., `cumulative_state += sum(new_partition) - decay`) reads "prior cumulative value" and is recursive. Rejected in v1 by the general "exactly one driving source" rule when the target itself is in the FROM clause. A future plan may admit this pattern with explicit input/state distinction.
- **Reprocessing via delta history.** A future plan may store per-partition deltas for cumulative models with reversible aggregators (`SUM`, `COUNT`, `BIT_XOR`), enabling subtract-then-add for reprocessing. The model's frontmatter does not declare reversibility; the classifier infers it from the projection list.
- **`--auto` staleness fidelity.** v1's staleness response for a cumulative model is conservative ("any partition ≥ the earliest stale"); the per-projection fidelity ("exactly the stale partitions for fully-reversible models") needs the delta-history mechanism above.
- **Schema evolution.** Adding a new non-key column to a cumulative table requires backfilling the new aggregator over the entire processed source history. v1 does not support this; the model must be rebuilt. Tracked under `schema_evolution.md` future work.
- **Sibling keyed modes.** `versioned` (SCD Type 2) and `latest_value` (SCD Type 1) are peers of cumulative on the refresh axis — keyed-output, smelt-maintained, derive-from-SQL — with their own specs (`versioned_models.md`, `latest_value_models.md`) and their own classifiers, not variants of this rule. Neither is implemented yet. The engine-maintained counterpart of any of them is hand-written SQL under `refresh: materialized_view` (`materialized_view.md`), not a maintainer flag on cumulative. Design of the sibling shapes: `docs/research/20260522-cumulative-as-its-own-rule.md` §"Sibling rules beyond cumulative_aggregate". A further sibling — **`accumulating_snapshot`**, a keyed once-write/milestone peer for retroactive enrichment (a row filled in by a later event, e.g. "did this event convert?") that consumes windowed input on the same driving-source axis as cumulative and bounds its forward-attribution horizon — is a normative peer with its own spec (`accumulating_snapshot.md`), not yet implemented. Worked design: `docs/research/20260703-model-updates.md` Part 20.
- **External sources without `timeseries:`.** A cumulative model whose only source is a non-timeseries external table has no partition shape to step over and is refused (`CumulativeNoDrivingSource`). The diagnostic suggests declaring `timeseries:` on the source.
- **Granularity restricted to `day` and `week`.** The v1 per-partition step loop accepts only `day` and `week` as the driving source's granularity. Any other granularity — `hour`, `month`, `quarter`, or `year` — is rejected at runtime with the error `cumulative_aggregate v1 supports day and week granularity; got <Granularity>`. This is a not-yet-supported limitation of the step loop arithmetic, not a permanent design boundary. Tracked in `docs/plans/20260611-docs-gap-remediation.md`.

## References

- **Code**:
  - `crates/smelt-core/src/config.rs` — the `Materialization` (storage) enum is `View | Table | Ephemeral` (target; `MaterializedView` is relocated to the refresh axis by the rename phase of `docs/plans/20260704-model-updates.md`); the refresh mode (`full` / `cumulative` / …) is a separate `refresh` axis; `IncrementalStrategy::Merge` (variant to drop)
  - `crates/smelt-core/src/metadata.rs` — frontmatter extraction, validation that `timeseries:` / a `batched:` block are absent when `refresh: cumulative`
  - `crates/smelt-logical/src/rules/cumulative.rs` — the cumulative classifier (pure rule-data, in `smelt-logical`; `smelt-planner` re-exports — see architecture.md §"Constraints & Invariants" (Layered single-ownership))
  - `crates/smelt-planner/src/rules/` — host for the per-partition step loop (rule *application*)
  - `crates/smelt-backend/src/lib.rs` — `merge_into` trait method (physical primitive the rule calls)
  - `crates/smelt-backend-duckdb/src/lib.rs` — DuckDB `merge_into` implementation
- **Tests**:
  - `crates/smelt-backend-duckdb/src/lib.rs::test_merge_into_upsert`, `test_merge_into_insert_only` — backend primitive coverage
  - Cumulative classifier unit tests and per-partition equivalence tests (to be added alongside the implementation plan)
- **User docs**: `docs-site/docs/guide/materializations.md` (to be updated to document `refresh: cumulative` on the refresh axis, alongside `batched`)
- **Plans (history)**:
  - `docs/plans/20260523-cumulative-aggregate.md` — implementation plan derived from this spec
- **Research**:
  - `docs/research/20260522-cumulative-as-its-own-rule.md` — the rationale for the rule shape; predecessor of this spec
  - `docs/research/20260521-incremental-as-planner-rule.md` — sibling research; the "derive from SQL, not YAML" principle this spec inherits
  - `docs/research/2026-05-20-incremental-gaps-from-web-analytics.md` — Gap #5, the original motivation
- **Related specs**:
  - `model_maintenance.md` — owns the processed-input equivalence invariant (end-state specialisation) and the algebraic maintenance ladder this mode cites; cumulative is their reference implementation
  - `model_properties.md` — owns the algebraic discriminants, driving-fact resolution, and monotonicity trace this mode requires
  - `model_transforms.md` — owns the keyed `merge_into`, windowed-keyed-maintenance driver, source-filter pushdown, and higher-rung transforms this mode drives
  - `batched_models.md` — the partitioned-output peer (per-partition equivalence, timeseries output)
  - `versioned_models.md`, `latest_value_models.md`, `materialized_view.md` — the other keyed-output refresh modes
  - `timeseries.md` — the source-side declaration this rule consumes
  - `models.md` — the three axes (kind / storage / refresh); refresh-axis and storage-enum host; frontmatter table
  - `expansion.md` — function expansion pass; runs before the classifier
  - `architecture.md` — `smelt.<path>` addressing; backend primitive contract
