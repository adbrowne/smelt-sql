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
