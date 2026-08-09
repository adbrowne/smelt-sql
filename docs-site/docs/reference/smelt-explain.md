# smelt explain

Inspect the logical and physical execution plan for a project, or the derived maintenance plan for a single incremental model.

```bash
smelt explain [MODEL_NAME] [--json] [--select <selector>] [--project-dir <path>] [--show-sql] [--period <start>..<end>] [--technique <name>]
```

## Options

| Flag | Description |
|---|---|
| `MODEL_NAME` | Optional. Name of a single model to print the maintenance-plan report for instead of the whole-project graph. |
| `--json` | Output as JSON instead of human-readable text. With `MODEL_NAME`, emits the per-model maintenance-plan report as JSON — with or without `--show-sql`, and with the same schema either way. |
| `--select` | Select models to include (repeatable). Ignored when `MODEL_NAME` is given. |
| `--project-dir` | Path to the smelt project root. Defaults to the current directory. |
| `--show-sql` | With `MODEL_NAME`, also print the maintenance statements each cell executes. Never connects to a backend. |
| `--period` | With `--show-sql`, real literal date bounds (`<start>..<end>`, end exclusive) for the printed statements' region. Without it, the symbolic placeholders `{{window_start}}`/`{{window_end}}` stand in. |
| `--technique` | Requires `--show-sql`. Render a named technique's own preview statements instead of the admitted one's, per cell — including a `NotApplicable` reason where that technique doesn't apply to a given cell. Accepts `delete_insert`, `keyed_fold`, `column_scoped_merge`, `in_place_update`, `per_group_recompute`, `recompute`. |

## Human-readable output

Without `--json` or a `MODEL_NAME`, smelt prints a summary of the logical graph (models, dependencies, materialization, incremental config) followed by the physical graph (planner optimisations, execution order, per-model strategy).

## Per-model maintenance plan

`smelt explain <model>` prints that model's derived **maintenance plan** instead of the whole-project graph: every cell (trigger, corner, technique), the `ledger_catch_up` flag (whether the cell routes through the [reconciliation ledger](../guide/incremental-models.md#the-reconciliation-ledger)), the derived per-source scan clamps, each source's partition-locality verdict, any admission refusals, and the model's inbound propagation edges. This only applies to `refresh: incremental` models with a `grain:` declared — other models print a one-line notice instead.

Add `--show-sql` to also print, after each cell's block, the maintenance statements that cell executes — the output of the same pure emitters a run executes. Each cell's SELECT body is compiled through the real discovered project's ephemeral resolver and upstream column types, the same way a run compiles it, so the printed SQL matches what a run would compile (referenced ephemeral models are CTE-inlined, and ref-column aggregates cast to their real type). A transactional group (e.g. a paired region `DELETE`+`INSERT`) is bracketed by `BEGIN`/`COMMIT` lines. `--show-sql` never connects to a backend. Combine with `--period <start>..<end>` for the real literal window bounds, or omit it to see the symbolic `{{window_start}}`/`{{window_end}}` placeholders instead. Combined with `--json`, the report is emitted as JSON with a `statements` array per cell.

Add `--technique <name>` alongside `--show-sql` to preview a *different* technique than the one the plan admitted — every technique smelt has an emitter for, rendered against each cell's own contract/identity/column data and labelled with its admissibility: `Admitted` (the plan's own choice), `InterchangeableAlternative` (proven sound here, but not the one picked — region recompute always qualifies when it isn't itself admitted), or `NotApplicable` with a reason (the technique's preconditions aren't met for this cell — reported, never omitted). Accepts `delete_insert`, `keyed_fold`, `column_scoped_merge`, `in_place_update`, `per_group_recompute`, `recompute` (`recompute` and `delete_insert` are the same technique). The preview always uses the symbolic `{{window_start}}`/`{{window_end}}` placeholders regardless of `--period` — it's a display-only illustration, not a windowed dry run. `--json` output always carries every technique's preview per cell (a `technique_previews` array) plus the model's full derived property set, whether or not `--technique` is given.

One narrow gap: a column aggregated directly off an ephemeral ref (rather than a materialized upstream model) still casts to the `BIGINT` default — a compile-order limitation shared identically by a real run, not an `explain`-specific divergence. See `docs/specs/cli.md` Known Divergences.

## Internal state columns

Some presented columns (`AVG`, the `STDDEV_*`/`VAR_*` family, `MAX_BY`/`MIN_BY`, and the fallback-bearing or multi-candidate once-write spellings) don't fold their presented value directly — they fold hidden **state columns** instead, and recompute the presented value from that state on every read. `smelt explain <model>` lists these state columns as internal state, distinct from the model's public schema:

```
State columns:
  - avg_amount (presented) folds through: avg_amount__sum, avg_amount__count
      presentation: avg_amount__sum / avg_amount__count
```

A model with no decomposed-state columns prints no state section. With `--json`, the same information appears as a top-level `state_columns` array: `[{"presented_column": "avg_amount", "state_columns": ["avg_amount__sum", "avg_amount__count"], "presentation_expr": "avg_amount__sum / avg_amount__count"}]`.

See [`smelt explain` in the CLI reference](cli.md#smelt-explain) for the full flag list and a sample maintenance-plan report. The web UI's [model diagnostics page](../guide/model-diagnostics.md) renders the same technique previews and admissibility verdicts interactively, alongside the model's full derived property set.

## JSON output schema

With `--json`, smelt prints a single JSON object with the following top-level fields:

```json
{
  "models": { "<model-name>": { ... }, ... },
  "execution_order": ["<model-name>", ...],
  "physical": { ... }
}
```

### `models` — per-model entry

Each entry in `models` has:

| Field | Type | Description |
|---|---|---|
| `dependencies` | `string[]` | Upstream model names. |
| `materialization` | `string` | Resolved storage materialization (`"view"`, `"table"`, `"ephemeral"`, `"materialized_view"`). |
| `refresh` | `string` | Resolved refresh strategy: `"incremental"` or `"materialized_view"`. Omitted when `"full"`. |
| `incremental` | object | Present only when the model is incremental. See below. |
| `tags` | `string[]` | Model tags from frontmatter or `smelt.yml`. Omitted when empty. |
| `owner` | `string` | Model owner from frontmatter `owner:` key. Omitted when absent. |
| `origin` | object | Present only for generator-emitted models. Contains `type`, `generator_file`, `generator_name`. |

### `incremental` object

Present on a model when `refresh: incremental` and `grain:` are set and a `timeseries:` block is declared.

| Field | Type | Description |
|---|---|---|
| `granularity` | `string` | Partition granularity (`"day"`, `"week"`, etc.). |
| `partition_column` | `string` | The output column used as the partition key. |
| `event_time_column` | `string` | The source timestamp column. |
| `unique_key` | `string[]` | Columns for MERGE-strategy deduplication. Omitted when empty. |
| `batch_safety` | `string` | Batch-safety classification. One of `"fully_batch_safe"`, `"bounded_safe(chunk=Nd,context=Nd)"`, `"per_partition_only"`. |
| `source_bounds` | object | Per-source bound map derived from the model's SQL. See below. Omitted when there are no timeseries upstream references. |

### `source_bounds` field

The `source_bounds` object maps each timeseries upstream reference name to its derived bound. Lookup sources (those without a `timeseries:` declaration) do not appear.

Each entry is a tagged object with `"type"`:

**`"bounded"`** — the planner derived an explicit lookback/lookahead:

```json
{
  "events_parsed": {
    "type": "bounded",
    "partition_col": "event_date",
    "before": "PT30M",
    "after": "PT0S"
  }
}
```

| Field | Type | Description |
|---|---|---|
| `partition_col` | `string` | The source's declared `timeseries.partition_column`. |
| `before` | `string` | ISO-8601 duration — how far before the run window to read the source. `"PT0S"` means partition-local (no extra lookback). |
| `after` | `string` | ISO-8601 duration — how far after the run window to read the source. |

**`"unbounded"`** — the source requires reading unbounded history (cumulative aggregation, `UNBOUNDED PRECEDING`):

```json
{ "type": "unbounded" }
```

**`"not_derivable"`** — the planner could not determine the bound (bare `LAG/LEAD` without a `RANGE` clause, or a computed-expression join without an explicit interval filter). A model with any `not_derivable` bound is refused at planning time.

```json
{ "type": "not_derivable" }
```

### Duration format

All `before` and `after` values use ISO-8601 duration strings:

| Value | Duration |
|---|---|
| `"PT0S"` | Zero — partition-local read. |
| `"PT30M"` | 30 minutes. |
| `"PT2H"` | 2 hours. |
| `"P1D"` | 1 day. |
| `"P7D"` | 7 days. |

### `physical` object

| Field | Type | Description |
|---|---|---|
| `execution_order` | `string[]` | Order the physical nodes are executed. |
| `nodes` | object | Per-node metadata. |
| `ephemerals` | `string[]` | Logical models inlined as CTEs. |
| `transformations` | `string[]` | Human-readable planner optimisation descriptions. |

Each node in `nodes` has `strategy`, `materialization`, `target`, and `logical_origins`.

## Example

```bash
# Human-readable
smelt explain

# JSON, piped to jq for the sessions model's source bounds
smelt explain --json | jq '.models.sessions.incremental.source_bounds'
```
