---
feature: timeseries
status: experimental
last_reviewed: 2026-05-21
owners: [andrew]
---

# Timeseries

> **Scope.** A normative spec for the `timeseries:` frontmatter block — the declaration of a time dimension on a model's or source's output. Out of scope: incremental execution (see `incremental_models.md`), source YAML grammar beyond the timeseries block (see `sources.md`), full model frontmatter schema (see `models.md`).
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
| `TimeseriesRequiredForIncremental` | Error | A model declares `incremental:` without `timeseries:`. |

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

### Compatibility with materialization modes

| Materialization | `timeseries:` allowed? |
|---|---|
| `view` | Yes — a view can declare a time dimension; downstream rules may push down. |
| `table` | Yes. |
| `materialized_view` | Yes. |
| `ephemeral` | No — ephemeral models have no persisted output; declaring `timeseries:` is `MalformedTimeseries`. |
| `test` | No — test models are not persistent outputs; declaring `timeseries:` is `MalformedTimeseries`. |

### Interaction with `incremental:`

A model that declares `incremental:` (per `incremental_models.md`) must also declare `timeseries:`. The two blocks have independent surfaces — `timeseries:` declares the time dimension, `incremental:` opts the model into incremental execution and carries strategy-specific keys (`unique_key`, `safety_overrides`, etc.). Declaring `incremental:` without `timeseries:` is `TimeseriesRequiredForIncremental`.

A source declaring `timeseries:` opts in to being a pushdown target for downstream rules. It does not run incrementally — sources are externally managed.

### Validation rules

1. **Partition column projection.** For a model, `partition_column` must appear in the model's output `SELECT` list (and, if grouping is present, in the `GROUP BY`). For a source, `partition_column` must appear in the declared `columns:` list. Violation produces `MalformedTimeseries`.
2. **Event-time column projection.** For a model, `event_time_column` must appear in the model's output. For a source, it must appear in the declared `columns:` list. Violation produces `MalformedTimeseries`.
3. **Type constraint on event_time_column.** Must be a date, timestamp, or timestamp-with-timezone type per `types.md`. Violation produces `MalformedTimeseries`.
4. **Type constraint on partition_column.** Must be a date or integer type. (Date-typed partitions are the common case; integer-typed partitions support custom epoch-encoded forms.) Violation produces `MalformedTimeseries`.
5. **Granularity closure.** Must be one of the enumerated values. Unknown values produce `MalformedTimeseries`. Custom granularity is reserved for a future plugin surface (see `incremental_models.md` § "Granularity values").
6. **`week_start` requires `granularity: week`.** Setting `week_start` on any other granularity is `MalformedTimeseries`.

### LSP surface

- **Hover** on a `smelt.<path>` reference whose target carries `timeseries:` shows the declared partition column and granularity alongside the column list.
- **Diagnostics** for `MalformedTimeseries` and `TimeseriesRequiredForIncremental` follow the standard diagnostic format (`lsp.md`).
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

## Constraints & Invariants

1. **One declaration site per entity.** A given model or source declares `timeseries:` in exactly one place (frontmatter or `smelt.yml`, merged key-by-key). Two frontmatter blocks would already be rejected by YAML; cross-source duplicates with conflicting values are a configuration error.
2. **`timeseries:` is additive, never subtractive.** A `smelt.yml` override cannot remove a key declared in frontmatter. (Frontmatter wins; merging is additive.)
3. **Granularity is closed.** The set of valid values is fixed at the spec level. A `granularity:` value outside the enum is `MalformedTimeseries`.
4. **`partition_column` must exist on the output / in the columns list.** No "virtual" or "synthesised" partition column for a v1 timeseries declaration. The column must be projected.
5. **Sources never run; `timeseries:` on a source does not imply any execution behaviour.** Sources remain externally managed (`sources.md`). A `timeseries:` block on a source declares the partition shape for downstream pushdown only.
6. **Compatibility with ephemeral/test materializations is forbidden.** Out of scope and deliberately so (Semantics § "Compatibility with materialization modes").

## Known Divergences / Open Questions

- **Migration from nested `incremental: { event_time_column, partition_column, granularity, enabled }`.** Today's implementation reads the time-dimension fields from inside `incremental:`. The migration to the `timeseries:` block is the subject of an upcoming plan derived from `docs/research/20260521-incremental-as-planner-rule.md`. The implementation cuts over in one pass — no transitional dual-form support per the project's no-backward-compatibility doctrine.
- **`week_start` not yet implemented.** The field is specced as the home for the configurable week-start day; backend support is part of the same migration plan.
- **Custom granularity plugin surface.** Reserved for future work. No plugin shipping today; the closed enum is authoritative.
- **`smelt verify` against the database.** A future pass could check that an external source's declared `partition_column` and `event_time_column` exist in the live database. Out of scope here; mentioned in `sources.md` Known Divergences as well.
- **LSP enrichment.** Hover and goto-definition for `timeseries:` fields are specced (Semantics § "LSP surface") but not yet implemented in the LSP. Tracked alongside the migration plan.

## References

- **Code**:
  - `crates/smelt-core/src/config.rs` — `IncrementalConfig` (today's home of the fields; will be split into `TimeseriesConfig` + a slimmer `IncrementalConfig`)
  - `crates/smelt-core/src/metadata.rs` — frontmatter parsing (`ModelMetadata`)
  - `crates/smelt-core/src/sources.rs` — `SourceInfo` (will gain a `timeseries:` parser path)
- **Tests**: schema validation, `MalformedTimeseries` diagnostic coverage, `TimeseriesRequiredForIncremental` coverage (to be added with the migration plan)
- **User docs**: to be authored alongside the migration plan; will live at `docs-site/docs/reference/timeseries.md`
- **Plans (history)**:
  - `docs/research/20260521-incremental-as-planner-rule.md` — research doc that proposed factoring `timeseries:` out of `incremental:`
- **Related specs**:
  - `incremental_models.md` — consumes `timeseries:`; carries incremental-rule-specific keys
  - `sources.md` — host for `timeseries:` on external sources
  - `models.md` — host for `timeseries:` on model frontmatter; lists the key in the frontmatter table
  - `types.md` — the date/timestamp/integer type vocabulary used for type constraints
  - `smelt_yml.md` — the project-level override surface
