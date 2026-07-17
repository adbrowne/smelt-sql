---
feature: timeseries
status: experimental
last_reviewed: 2026-07-17
owners: [andrew]
---

# Timeseries

> **What this is.** A normative spec for the `timeseries:` frontmatter block — the declaration of a time dimension on a model's or source's output. Out of scope: incremental execution (see `incremental_models.md`), source YAML grammar beyond the timeseries block (see `sources.md`), full model frontmatter schema (see `models.md`).
>
> **Spec-first rule.** Edit this file before writing the implementation plan. The spec diff is the change description.
>
> **Timeless-oracle rule.** This spec describes the feature as if it has always existed. Implementation status that needs naming goes in §Known Divergences or §References → Plans (history). See `CLAUDE.md`.

## Surface

### Frontmatter block (in `.sql` models)

```sql
---
materialization: table
timeseries:
  event_time_column: order_ts        # the source-of-truth time column
  partition_column: order_date       # the column the engine prunes on
  granularity: day                   # hour | day | week | month | quarter | year
---

SELECT DATE_TRUNC('day', order_ts) AS order_date, order_ts, customer_id, amount
FROM smelt.orders_raw
```

| Key | Required | Default | Type / Values |
|---|---|---|---|
| `event_time_column` | yes | — | Identifier — name of the time column on the model's output (timestamp or date) |
| `partition_column` | yes | — | Identifier — name of the column the engine prunes on (date or integer) |
| `granularity` | yes | — | One of: `hour`, `day`, `week`, `month`, `quarter`, `year` |
| `week_start` | no | `monday` | When `granularity: week`. One of: `monday`, `sunday` |
| `assert_monotonic` | no | `false` | Boolean — the declared-monotonicity escape hatch: the modeller's assertion that the projected `event_time`/partition expression is monotone non-decreasing even where static analysis cannot decide it (an opaque function call). Widens only the *undecidable* verdict; a construct static analysis positively disproves (a constant/`NULL` seed, a row-nondeterministic function, a periodic/piecewise construct) is still refused (`model_properties.md` §"Event-time monotonicity trace") |

`event_time_column` and `partition_column` may be the same column. They differ when the source-of-truth time is a timestamp and the partition is a derived date.

### Block on external source YAML

The same block, with the same keys, on a source `.yml` file (`sources.md`):

```yaml
description: Raw orders feed.
columns:
  - { name: order_id,  type: INTEGER, nullable: false }
  - { name: order_ts,  type: TIMESTAMP, nullable: false }
  - { name: order_date, type: DATE, nullable: false }
  - { name: customer_id, type: INTEGER, nullable: false }
  - { name: amount, type: DECIMAL(18,2), nullable: false }
timeseries:
  event_time_column: order_ts
  partition_column: order_date
  granularity: day
```

A source that declares `timeseries:` must declare the named `event_time_column` and `partition_column` in its `columns:` list with date/timestamp-compatible types.

### Diagnostic codes (owned by this spec)

| Code | Severity | Trigger |
|---|---|---|
| `MalformedTimeseries` | Error | The `timeseries:` block parses but violates a structural rule (missing required key, unknown key, `granularity` not in the enum, `partition_column` absent from the model's output / source's columns, `event_time_column` has an incompatible type). |
| `TimeseriesRequiredForBatched` | Error | A model declares `refresh: incremental` + `grain: partition` without `timeseries:`. |

### `smelt.yml` (project-level overrides)

```yaml
models:
  daily_revenue:
    timeseries:
      event_time_column: order_ts
      partition_column: order_date
      granularity: day
```

Frontmatter wins over `smelt.yml` when both set the same field. Field-level merging within `timeseries:`: frontmatter and `smelt.yml` are merged key-by-key; declaring `granularity: day` in frontmatter and `event_time_column: order_ts` in `smelt.yml` yields a single combined block.

## Semantics

### What a timeseries declaration means

A model or source carrying `timeseries:` declares to the planner: *this output has a time dimension, partitioned by `partition_column` at `granularity` boundaries, with `event_time_column` as the source-of-truth time*. Downstream consumers — planner rules, the CLI, the LSP — may rely on this declaration when reasoning about the model's output.

A model or source **without** `timeseries:` is non-timeseries — it has no declared time dimension. Downstream rules that need partition information treat it as a lookup (read in full, no pushdown).

**`granularity` is the declared propagation grain, checked rather than derived.** For cross-model dependency propagation, `granularity` is each node's partition-axis grain — `incremental_models.md` §"The graph layer" defines a dependency edge as running between two partition axes whose grain is the declared `timeseries.granularity` of each node, never per-edge and never derived from the SQL. The SQL's own grouping (a `GROUP BY`/`date_trunc` at some cadence) is not the source of truth; it is only *checked* against the declaration by the grain-alignment proof (`model_properties.md` §"Grain-alignment check"), which reports `Aligned` or `NotAligned{reason}` and never substitutes a derived value for the declared one.

### Compatibility with materialization modes

| Materialization | `timeseries:` allowed? |
|---|---|
| `view` | Yes — a view can declare a time dimension; downstream rules may push down. |
| `table` | Yes. |
| `materialized_view` | Yes. |
| `ephemeral` | No — ephemeral models have no persisted output; declaring `timeseries:` is `MalformedTimeseries`. |
| `test` | No — test models are not persistent outputs; declaring `timeseries:` is `MalformedTimeseries`. |

### Interaction with the partition grain (`grain: partition`)

A model that declares `refresh: incremental` + `grain: partition` (the shape profile detailed in `incremental_models.md` §"The partition grain (`grain: partition`)") must also declare `timeseries:`. The two surfaces are independent — `timeseries:` declares the time dimension, the partition-grain surface carries grain-specific keys (`unique_key`, `safety_overrides`, etc.). Declaring `grain: partition` without `timeseries:` is `TimeseriesRequiredForBatched`.

A source declaring `timeseries:` opts in to being a pushdown target for downstream rules. It does not run incrementally — sources are externally managed.

### Interaction with the key grain (`grain: key`)

A `grain: key` model (the shape profile detailed in `incremental_models.md` §"The key grain (`grain: key`)") may declare `timeseries:` to time-partition its keyed output. Admission is gated on **key temporal locality**, owned by `incremental_models.md` §"Key temporal locality" — this spec owns only the block grammar and the structural rules below. A key-grain model without an admitted block has non-timeseries output (a lookup).

### Validation rules

1. **Partition column projection.** For a model, `partition_column` must appear in the model's output `SELECT` list (and, if grouping is present, in the `GROUP BY` — except on a `grain: key` model, where it may instead be an aggregate projection admitted by key temporal locality; `incremental_models.md` §"Key temporal locality"). For a source, `partition_column` must appear in the declared `columns:` list. Violation produces `MalformedTimeseries`.
2. **Event-time column projection.** For a model, `event_time_column` must appear in the model's output. For a source, it must appear in the declared `columns:` list. Violation produces `MalformedTimeseries`.
3. **Type constraint on event_time_column.** Must be a date, timestamp, or timestamp-with-timezone type per `types.md`. Violation produces `MalformedTimeseries`.
4. **Type constraint on partition_column.** Must be a date or integer type. (Date-typed partitions are the common case; integer-typed partitions support custom epoch-encoded forms.) Violation produces `MalformedTimeseries`.
5. **Granularity closure.** Must be one of the enumerated values. Unknown values produce `MalformedTimeseries`. Custom granularity is reserved for a future plugin surface (see `incremental_models.md` § "Granularity values").
6. **`week_start` requires `granularity: week` and must be `monday` or `sunday`.** Setting `week_start` on any other granularity, or setting it to a weekday other than `monday` or `sunday`, is `MalformedTimeseries`.
7. **Partition / pruning columns must be NOT NULL.** `partition_column` must be NOT NULL on the model's output (or declared `nullable: false` on the source's columns). When `event_time_column` drives pruning (it differs from `partition_column` and is the column a downstream rule filters on), it must be NOT NULL too. A NULL partition value silently escapes the half-open `>= start AND < end` pruning window — it is never deleted or re-inserted — which is a correctness hole for incremental execution. A nullable partition/pruning column is `MalformedTimeseries`.
8. **Sub-day granularity requires a timestamp-resolution partition type.** When `granularity` is `hour` (a sub-day unit), `partition_column` must be a timestamp-resolution type (timestamp or timestamp-with-timezone), not a plain `date` — a `DATE` cannot represent hour boundaries, so hour-granularity pruning against a `DATE` partition silently coarsens to whole days. A sub-day granularity paired with a `date` (or otherwise day-resolution) partition type is `MalformedTimeseries`.

### LSP surface

- **Hover** on a `smelt.<path>` reference whose target carries `timeseries:` shows the declared partition column and granularity alongside the column list.
- **Diagnostics** for `MalformedTimeseries` and `TimeseriesRequiredForBatched` follow the standard diagnostic format (`lsp.md`).
- **Goto-definition** for a `timeseries:` field navigates to the column declaration in the model's output or the source YAML.

### Granularity arithmetic

A run window `[start, end)` is aligned to `granularity` boundaries: `end - start` must be a positive integer multiple of `granularity`, and both `start` and `end` must fall on `granularity` unit boundaries. Partial-unit windows are rejected. The CLI and any planner rule that consumes a run window enforces this rule.

For `granularity: week`, the boundary depends on `week_start`. Default Monday.

## Design

This section captures the load-bearing rationale.

**Time-dimension is a property of the output, not of incremental execution.** Historically the time-dimension fields (`event_time_column`, `partition_column`, `granularity`) lived inside `incremental:`. That conflated two concepts: *this output has a time dimension* and *this model runs incrementally*. A view, a non-incremental rollup, or an external source can have a meaningful time dimension that downstream rules should push down on, without itself running incrementally. Factoring `timeseries:` out makes the time-dimension a first-class property; `incremental:` becomes the on/off flag plus incremental-specific options. *Keeping the fields inside `incremental:` and adding a parallel `timeseries:` block on non-incremental models* was rejected — two places to declare the same information drift.

**`event_time_column` separate from `partition_column`.** Many timeseries outputs project a timestamp (the source-of-truth time) and a derived date (the partition). They can be the same column, and frequently are; but separating them lets the SQL be honest about the relationship — the event time is a timestamp, the partition is `DATE_TRUNC('day', event_time)`. *Collapsing the two into a single column* was rejected because downstream type-aware operations (range joins, window frames over time) need the finer-grained timestamp, while partition pruning needs the coarser-grained date.

**Closed granularity enum, not free intervals.** `hour`, `day`, `week`, `month`, `quarter`, `year` cover the vast majority of partition cadences and give planner rules a finite set to reason about (chunk arithmetic, lookback derivation, source-filter pushdown). *Free `INTERVAL` granularity* (`granularity: "12 hours"`) was rejected for v1 because the planner's run-window alignment and chunking heuristics become open-ended — easier to extend the enum than to support arbitrary intervals. A custom-granularity plugin surface is reserved for future work but ships no plugins today.

**`timeseries:` lives in core, not in any planner rule.** Future planner rules — MERGE-strategy, snapshot, CDC, late-data audit — will all want to read the time dimension. Putting `timeseries:` in core means it is one declaration consumed by many rules. `incremental:` carries only the rule-specific surface; rules that don't ship as part of the default smelt distribution can still consume `timeseries:` without changing core.

**The clock slot of the Relation Contract.** `timeseries:` is the **clock** slot of the shared Relation Contract (`models.md` §"The Relation Contract") — the one field path both a source (fills by declaration) and a model output (fills declared-and-checked) carry **identically**, which is precisely what lets a downstream consumer window over an upstream maintained model exactly as over a source (`incremental_models.md` §"Upstream model edges"). The shared grammar on both providers is deliberate, not a coincidence. Together with the identity slot (`unique_key:`), the presence or absence of this clock is one of the two shape-defining facts from which a relation's `grain` label is *derived* — never a declared `grain:` token (`incremental_models.md` §"Grain is a derived label").

## Constraints & Invariants

1. **One declaration site per entity.** A given model or source declares `timeseries:` in exactly one place (frontmatter or `smelt.yml`, merged key-by-key). Two frontmatter blocks would already be rejected by YAML; cross-source duplicates with conflicting values are a configuration error.
2. **`timeseries:` is additive, never subtractive.** A `smelt.yml` override cannot remove a key declared in frontmatter. (Frontmatter wins; merging is additive.)
3. **Granularity is closed.** The set of valid values is fixed at the spec level. A `granularity:` value outside the enum is `MalformedTimeseries`.
4. **`partition_column` must exist on the output / in the columns list.** No "virtual" or "synthesised" partition column for a v1 timeseries declaration. The column must be projected.
5. **Sources never run; `timeseries:` on a source does not imply any execution behaviour.** Sources remain externally managed (`sources.md`). A `timeseries:` block on a source declares the partition shape for downstream pushdown only.
6. **Compatibility with ephemeral/test materializations is forbidden.** Out of scope and deliberately so (Semantics § "Compatibility with materialization modes").
7. **Partition and pruning columns are NOT NULL.** `partition_column` (and `event_time_column` when it drives pruning) must be NOT NULL on the output / source. A nullable pruning column silently escapes the `>= start AND < end` window and is rejected with `MalformedTimeseries` (Semantics § "Validation rules", rule 7).
8. **Granularity resolution must fit the partition type.** A sub-day granularity (`hour`) requires a timestamp-resolution `partition_column`; pairing it with a day-resolution type (`date`) is `MalformedTimeseries` (Semantics § "Validation rules", rule 8).

## Known Divergences / Open Questions

- **Migration from nested `incremental: { event_time_column, partition_column, granularity, enabled }`.** Today's implementation reads the time-dimension fields from inside `incremental:`. The migration to the `timeseries:` block is the subject of an upcoming plan derived from `docs/research/20260521-incremental-as-planner-rule.md`. The implementation cuts over in one pass — no transitional dual-form support per the project's no-backward-compatibility doctrine.
- **Custom granularity plugin surface.** Reserved for future work. No plugin shipping today; the closed enum is authoritative.
- **`smelt verify` against the database.** A future pass could check that an external source's declared `partition_column` and `event_time_column` exist in the live database. Out of scope here; mentioned in `sources.md` Known Divergences as well.
- **LSP enrichment.** Hover and goto-definition for `timeseries:` fields are specced (Semantics § "LSP surface") but not yet implemented in the LSP. Tracked alongside the migration plan.
- **Output-schema-dependent validation rules (rules 2, 3, 4).** Validation rules 2 (event-time column projection), 3 (event-time type constraint), and 4 (partition-column type constraint) require the model's output schema. These rules land alongside the R2 incremental-cadence rewrite; tracked in `docs/plans/20260521-incremental-timeseries-and-derived-bounds.md`. Rules 7 and 8 (NOT-NULL invariant and sub-day granularity type) are implemented in `smelt-db`. Rules 1, 5, 6 are enforced at frontmatter parse time.

## References

- **Code**:
  - `crates/smelt-core/src/config.rs` — `BatchedConfig`, `TimeseriesConfig`
  - `crates/smelt-core/src/metadata.rs` — frontmatter parsing (`ModelMetadata`)
  - `crates/smelt-core/src/sources.rs` — `SourceInfo` (will gain a `timeseries:` parser path)
- **Tests**: schema validation, `MalformedTimeseries` diagnostic coverage, `TimeseriesRequiredForBatched` coverage (to be added with the migration plan)
- **User docs**: to be authored alongside the migration plan; will live at `docs-site/docs/reference/timeseries.md`
- **Plans (history)**:
  - `docs/research/20260521-incremental-as-planner-rule.md` — research doc that proposed factoring `timeseries:` out of `incremental:`
- **Related specs**:
  - `incremental_models.md` — consumes `timeseries:`; carries grain-specific keys
  - `sources.md` — host for `timeseries:` on external sources
  - `models.md` — host for `timeseries:` on model frontmatter; lists the key in the frontmatter table
  - `types.md` — the date/timestamp/integer type vocabulary used for type constraints
  - `smelt_yml.md` — the project-level override surface
