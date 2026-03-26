# How smelt Works

## What is smelt

smelt is a SQL-to-SQL compiler and orchestrator for data transformation pipelines. You write SQL models that reference each other using `smelt.ref()`, and smelt takes care of the rest: resolving dependencies, compiling to target-specific SQL, and executing against your database.

```sql
-- models/marts/active_users.sql
---
materialization: table
---
SELECT
    user_id,
    COUNT(*) AS login_count
FROM smelt.ref('staging.user_logins')
WHERE login_date >= CURRENT_DATE - INTERVAL '30 days'
GROUP BY user_id
```

smelt parses this SQL, understands that `active_users` depends on `staging.user_logins`, builds a dependency graph, and executes models in the correct order.

## Logical vs Physical Separation

The core architectural insight behind smelt is the separation of **what to compute** from **how to execute**.

**Logical layer (what you write):**

- SQL models with `smelt.ref()` calls
- YAML frontmatter for configuration
- Pure data transformation logic

**Physical layer (what smelt decides):**

- Which database engine to use
- Whether to materialize as a table, view, or materialized view
- Whether to run incrementally or as a full refresh
- How to batch and partition the work
- What optimizations to apply across model boundaries

This separation has practical consequences:

- **You write pure SQL**, not execution logic. No `{% if is_incremental() %}` branching.
- **The framework can automatically incrementalize models** by analyzing your SQL and generating the appropriate DELETE+INSERT logic.
- **The same models run on different backends.** Use DuckDB locally for fast iteration, then deploy to Spark or PostgreSQL in production without changing your SQL.
- **Optimizations are applied automatically** without modifying user code. smelt can fuse queries, push down predicates, and reorganize execution across model boundaries.

## The Compilation Pipeline

Every `smelt run` follows this pipeline:

```
SQL Files --> Parse --> Analyze --> Plan --> Generate --> Execute
```

**Parse**
:   The error-recovery parser produces a complete syntax tree even from incomplete or invalid SQL. This powers both the CLI and the LSP -- you get diagnostics and autocompletion while typing, not just at build time.

**Analyze**
:   Resolve `smelt.ref()` and `smelt.source()` calls to their targets. Infer column types. Validate schemas and detect mismatches. Build the dependency graph.

**Plan**
:   Determine execution order from the dependency graph. Detect optimization opportunities (predicate pushdown, model fusion). Choose materialization strategies. For incremental models, generate the appropriate merge logic.

**Generate**
:   Compile the analyzed SQL into the target engine's dialect. DuckDB, Spark SQL, and PostgreSQL each have syntax differences that smelt handles transparently.

**Execute**
:   Send the generated SQL to the target engine. Track run history and interval state for incremental models. Report results.

## How smelt Differs from dbt

### No Jinja templates

dbt embeds execution logic in SQL files using Jinja:

```sql
-- dbt
{{ config(materialized='incremental') }}

SELECT * FROM {{ ref('events') }}
{% if is_incremental() %}
WHERE event_date > (SELECT MAX(event_date) FROM {{ this }})
{% endif %}
```

smelt parses and understands your SQL semantically. Configuration goes in YAML frontmatter, and incrementalization is handled by the framework:

```sql
-- smelt
---
materialization: table
incremental:
  enabled: true
  event_time_column: event_date
  partition_column: event_date
  granularity: day
---
SELECT * FROM smelt.ref('events')
```

### Automatic incrementalization

smelt analyzes your SQL to determine whether it is safe to run incrementally. It detects partition columns, validates safety constraints (no window functions with unbounded frames, no LIMIT, no non-deterministic functions), and generates DELETE+INSERT logic automatically.

You declare *intent* (enable incremental with a partition column), and smelt handles the *mechanics*.

### Static type checking

The smelt LSP catches errors before you run anything:

- Undefined `smelt.ref()` targets
- Column type mismatches between models
- Schema validation against source definitions
- Parse errors with precise line/column positions

### Multi-backend execution

A single smelt project can target multiple engines. Define targets in `smelt.yml` and switch between them:

```bash
smelt run --target dev      # DuckDB on your laptop
smelt run --target prod     # Spark on your cluster
```

The same SQL models work across both -- smelt generates the appropriate dialect for each target.

## Key Concepts

Model
:   A SQL file in `models/` that defines a data transformation. Each model produces one table or view.

Ref
:   `smelt.ref('model_name')` creates a dependency on another model. smelt resolves these to actual table names and ensures correct execution order.

Source
:   `smelt.source('schema.table')` references an external table defined in `sources.yml`. Sources are not managed by smelt but are validated for schema correctness.

Materialization
:   How a model's results are persisted in the database: `table` (full rebuild), `view` (virtual), `ephemeral` (inlined into downstream queries), or `materialized_view`.

Incremental
:   A materialization strategy that processes only new or changed data instead of rebuilding the entire table. smelt determines incrementalization safety automatically.

Seed
:   A CSV file in `seeds/` that is loaded into the database as a table. Useful for lookup data, mappings, and small reference tables.

Target
:   A named execution environment defined in `smelt.yml`. Each target specifies a database backend and connection details (e.g., `dev` for local DuckDB, `prod` for Spark).

Selector
:   A pattern for choosing which models to run. Use `--select model_name` for a single model, `--select +model_name` to include upstream dependencies, or `--select tag:finance` to run all models with a given tag.
