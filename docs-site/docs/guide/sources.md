# Sources

Sources represent external tables that are not managed by smelt -- they already exist in your database and are loaded outside of the smelt pipeline. Typical examples include raw data tables populated by ingestion tools, third-party datasets, or tables managed by other systems.

## Defining sources

Sources are declared in a `sources.yml` file at the root of your smelt project. The file maps schema names to tables, and each table can list its columns with types.

```yaml
version: 1

sources:
  raw:
    tables:
      customers:
        description: "Customer dimension table"
        columns:
          - name: customer_id
            type: INTEGER
          - name: name_prefix
            type: VARCHAR
          - name: country
            type: VARCHAR
          - name: signup_date
            type: VARCHAR
          - name: segment
            type: VARCHAR

      orders:
        description: "Order fact table"
        columns:
          - name: order_id
            type: INTEGER
          - name: customer_id
            type: INTEGER
          - name: order_date
            type: VARCHAR
          - name: status
            type: VARCHAR
```

The top-level key under `sources` is the schema name (`raw` in this example). Each table is listed under `tables` with an optional `description` and a list of `columns`.

## Using sources in models

Reference a source in your SQL models with `smelt.source('schema.table')`:

```sql
-- models/stg_customers.sql
SELECT
    customer_id,
    COALESCE(name_prefix, '') AS name_prefix,
    CAST(signup_date AS DATE) AS signup_date,
    COALESCE(segment, 'Unknown') AS segment
FROM smelt.source('raw.customers')
```

The argument is always `'schema.table'` -- the schema name and table name separated by a dot.

## Column declarations

Declaring columns in `sources.yml` serves two purposes:

1. **LSP completions** -- The language server uses column definitions to provide autocomplete suggestions as you write queries.
2. **Type checking** -- smelt can verify that your models reference columns that actually exist in the source and use compatible types.

!!! tip
    Even though column declarations are optional, adding them pays off quickly. The LSP will catch typos and type mismatches before you ever run a query.

## Multiple schemas

You can define sources across multiple schemas in the same file:

```yaml
version: 1

sources:
  raw:
    tables:
      users:
        columns:
          - name: user_id
            type: INTEGER
          - name: user_name
            type: VARCHAR

  analytics:
    tables:
      page_views:
        columns:
          - name: view_id
            type: INTEGER
          - name: user_id
            type: INTEGER
          - name: page_url
            type: VARCHAR
```

Reference them as `smelt.source('raw.users')` and `smelt.source('analytics.page_views')` respectively.

## Loading source data

smelt does not load source data for you. You are responsible for ensuring the source tables exist in your target database before running models that depend on them.

A common pattern is to use a setup script. For example, the timeseries example project includes a `setup_sources.sql` file that creates and populates the raw tables:

```sql
-- setup_sources.sql (run manually or via your ingestion pipeline)
CREATE TABLE raw.users AS SELECT * FROM read_csv('data/users.csv');
CREATE TABLE raw.events AS SELECT * FROM read_csv('data/events.csv');
```

## Further reading

- [Sources Configuration Reference](../reference/sources-yml.md) for the full `sources.yml` schema
- [SQL Models](sql-models.md) for how to write models that reference sources
