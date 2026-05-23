# smelt explain

Inspect the logical and physical execution plan for a project.

```bash
smelt explain [--json] [--project-dir <path>]
```

## Options

| Flag | Description |
|---|---|
| `--json` | Output as JSON instead of human-readable text. |
| `--project-dir` | Path to the smelt project root. Defaults to the current directory. |

## Human-readable output

Without `--json`, smelt prints a summary of the logical graph (models, dependencies, materialization, incremental config) followed by the physical graph (planner optimisations, execution order, per-model strategy).

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
| `materialization` | `string` | Resolved materialization (`"view"`, `"table"`, `"ephemeral"`). |
| `incremental` | object | Present only when the model is incremental. See below. |
| `tags` | `string[]` | Model tags from frontmatter or `smelt.yml`. Omitted when empty. |
| `owner` | `string` | Model owner from frontmatter `owner:` key. Omitted when absent. |
| `origin` | object | Present only for generator-emitted models. Contains `type`, `generator_file`, `generator_name`. |

### `incremental` object

Present on a model when `incremental: enabled: true` is set and a `timeseries:` block is declared.

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
