# Sources

Sources represent external tables that are not managed by smelt — they already exist in your database and are loaded outside of the smelt pipeline. Typical examples include raw data tables populated by ingestion tools, third-party datasets, or tables managed by other systems.

## Defining sources

A source is declared by placing a `.yml` file under any directory listed in `paths:` in your `smelt.yml`. The file must not share a stem with a sibling `.csv` (that would make it a seed sidecar instead).

```yaml
# models/sources/raw/users.yml
description: Raw user dimension; populated nightly by the CDC pipeline.
columns:
  - name: user_id
    type: INTEGER
    nullable: false
  - name: user_name
    type: VARCHAR
  - name: signup_date
    type: DATE
```

The address of this source is derived from its path under the scan root. With `paths: ["models"]`, the file `models/sources/raw/users.yml` resolves to `smelt.sources.raw.users`.

## Using sources in models

Reference a source in your SQL models using its full `smelt.<path>` address:

```sql
-- models/staging/stg_users.sql
SELECT
    user_id,
    user_name,
    CAST(signup_date AS DATE) AS signup_date
FROM smelt.sources.raw.users
```

## Overriding the database name

By default smelt maps `smelt.sources.raw.users` to `<target_schema>.sources_raw_users`. When the external table has a different name, use the `name:` override:

```yaml
# models/sources/raw/users.yml
name: warehouse.users_v2
columns:
  - name: user_id
    type: INTEGER
```

With `name: warehouse.users_v2` set, smelt emits `FROM warehouse.users_v2` in compiled SQL instead of the default mapping.

## Column declarations

Declaring columns serves two purposes:

1. **LSP completions** — the language server uses column definitions to provide autocomplete suggestions as you write queries.
2. **Type checking** — smelt can verify that your models reference columns that actually exist in the source and use compatible types.

`columns:` is required on a source (unlike seed sidecars, where it is optional).

!!! tip
    Even though smelt cannot verify the source exists in the database, adding column declarations lets it catch typos and type mismatches before you run a query.

## Loading source data

smelt does not load source data. You are responsible for ensuring the source tables exist in your target database before running models that depend on them.

## Project structure

Source YAML files live alongside other models under your `paths:` directories. A typical layout:

```
models/
  sources/
    raw/
      users.yml
      events.yml
      transactions.yml
  staging/
    stg_users.sql
    stg_events.sql
```

## Further reading

- [Sources YAML Reference](../reference/sources-yml.md) for the full per-entity YAML schema
- [SQL Models](sql-models.md) for how to write models that reference sources
