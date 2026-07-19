# Source YAML Reference

Each source is declared as a single `.yml` file placed under any directory listed in `paths:` in `smelt.yml`. The file must not share its stem with a sibling `.csv` (that would make it a seed sidecar instead).

## File placement and addressing

| File on disk (with `paths: ["models"]`) | Address |
|---|---|
| `models/sources/raw/users.yml` | `smelt.sources.raw.users` |
| `models/external/api/orders.yml` | `smelt.external.api.orders` |

The address follows universal path addressing: the scan-root prefix (`models/`) is stripped, the directory path and stem become the address segments.

A `smelt.sources.<path>` reference always resolves under the sources namespace; a model whose name happens to collide with a leaf segment does not shadow the source.

## Top-level keys

| Key | Required | Default | Meaning |
|-----|----------|---------|---------|
| `description` | no | absent | Free-text description of the table, surfaced in LSP hover. |
| `columns` | **yes** | — | Column declarations. Required; a source without `columns:` has no contract to type-check. |
| `name` | no | derived | Override the database-side name. Must be a `<schema>.<table>` literal. When absent, defaults to `<target_schema>.<address-path-joined-by-_>`. |
| `materialization` | **forbidden** | — | Not allowed. Sources are externally managed. Produces a hard error. |
| `timeseries` | no | absent | Declares the source's time dimension (`event_time_column`, `partition_column`, `granularity`) — same shape as a model's `timeseries:` block. See [Timeseries reference](timeseries.md). |
| `unique_key` | no | absent | Identity column(s) — a string or list. Required alongside `referential_integrity` (a subset of it), and consumed as the once-write/functional-dependency identity a downstream `grain: key` model's [key temporal locality](timeseries.md#interaction-with-grain-key) can key off. |
| `mutation_profile` | no | absent (treated as unclocked/worst-case) | How this source's underlying data changes over time — `append_only`, `mutable_snapshot`, or `change_feed`, bare-string or structured (see [Mutation profile](#mutation-profile) below). |
| `source_lateness` | no | absent | Interval (e.g. `'2 hours'`) declaring how far behind "now" this source's data may lag; folded into a downstream model's read window. |
| `referential_integrity` | no | absent | Column(s) — a subset of `unique_key` — guaranteed to have a matching row for every value a consuming model's inner-join enrichment reads by. Narrows an enrichment `MERGE`'s recompute to a point lookup on the changed key(s); re-checked every run, never trusted silently. See [Declaring referential integrity](../guide/sources.md#declaring-referential-integrity). |

## Column keys

| Key | Required | Default | Meaning |
|-----|----------|---------|---------|
| `name` | yes | — | Column name as it appears in the database. |
| `type` | yes | — | smelt `DataType` — see [Supported types](#supported-types). |
| `nullable` | no | `true` | Whether the column may contain NULL. `nullable: false` is a guarantee that type inference carries downstream. When the column appears on the null-supplying side of an outer join (`LEFT JOIN` right side, `RIGHT JOIN` left side, either side of `FULL JOIN`), inference overrides the declared guarantee and marks the column nullable in the join's output scope. |
| `description` | no | absent | Free-text description, surfaced in LSP hover. |

## Complete example

```yaml
# models/sources/raw/users.yml
description: Raw user dimension; populated nightly by the CDC pipeline.
columns:
  - name: user_id
    type: INTEGER
    nullable: false
    description: Surrogate key.
  - name: user_name
    type: VARCHAR
  - name: signup_date
    type: DATE
```

## With `name:` override

```yaml
# models/sources/raw/users.yml
name: warehouse.users_v2
description: Canonical user table; external name differs from workspace path.
columns:
  - name: user_id
    type: INTEGER
    nullable: false
  - name: user_name
    type: VARCHAR
```

With `name: warehouse.users_v2`, smelt emits `FROM warehouse.users_v2` in compiled SQL instead of the default `<target_schema>.sources_raw_users`.

## Mutation profile

```yaml
mutation_profile:
  kind: append_only          # append_only | mutable_snapshot | change_feed
  lateness: '3 days'         # append_only only
  redelivery: at_least_once  # append_only only
  key_recurrence:            # any kind
    key: [event_id]
    window: '3 days'
```

`mutation_profile` accepts a bare string (`mutation_profile: change_feed`) as shorthand for `{ kind: change_feed }`, or the structured block above for `kind:` plus sub-facts scoped to it:

| Field | Applies to `kind:` | Meaning |
|---|---|---|
| `kind` | any | `append_only`, `mutable_snapshot`, or `change_feed`. |
| `lateness` | `append_only` | How late a row for an already-passed partition may still arrive. |
| `redelivery` | `append_only` | Redelivery posture — e.g. `at_least_once` for a feed that may redeliver a row identically. |
| `retractions`, `ordered`, `delta_identity` | `change_feed` | Feed-shape facts describing how the change feed itself is structured. |
| `key_recurrence.key` / `key_recurrence.window` | any | The declared **recurrence bound**: every pair of rows sharing the named key(s) lies within `window` of each other on the event-time axis. This is route 3 (recurrence-bounded) of a downstream `grain: key` model's [key temporal locality](timeseries.md#interaction-with-grain-key) — a checked declaration, not a proof: violations fail the consuming run transactionally (`KeyedRecurrenceBoundViolated`) rather than silently producing a wrong answer. See the [deduplication tutorial](../examples/web-analytics/deduplication.md) for a worked example. |

## Supported types

| Type | Description |
|------|-------------|
| `INTEGER` | 64-bit signed integer |
| `DECIMAL(p,s)` | Fixed-point decimal, precision `p`, scale `s` |
| `DOUBLE` | 64-bit floating point |
| `BOOLEAN` | True/false |
| `DATE` | Calendar date (`YYYY-MM-DD`) |
| `TIMESTAMP` | Date and time without time zone |
| `VARCHAR` | Variable-length string |
