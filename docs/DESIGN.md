# smelt Design Specification

## Overview

smelt is a data transformation framework that separates **logical transformation definitions** from **physical execution planning**. Unlike traditional tools like dbt that use SQL templates, smelt parses and understands the semantics of transformations, enabling advanced capabilities like automatic refactoring, cross-engine deployment, and intelligent incrementalization.

### Core Philosophy

1. **Analysts define what** - Pure logical models expressing business intent
2. **Engineers define how** - Rewrite rules and execution configuration
3. **Framework mediates** - Validates, optimizes, and deploys to target engines

### Design Principles

#### ✅ Keep Logical Models Pure

- **DO**: Write SQL that expresses business logic
- **DO**: Use `smelt.ref()` and `smelt.metric()` extensions
- **DO**: Add configuration via YAML frontmatter (annotation syntax like `@materialize` is a future consideration; YAML frontmatter is the current config surface)
- **DON'T**: Add macros, conditionals, or templating to logical models
- **DON'T**: Mix execution strategy with business logic

#### ✅ Backends Own Computational State

- **DO**: Let Delta Lake, Flink, DuckDB manage watermarks and checkpoints
- **DO**: Query backends for current table state when needed
- **DO**: Trust backend transaction logs and state management
- **DON'T**: Store watermarks, stream offsets, or batch boundaries in smelt
- **DON'T**: Duplicate state that backends already manage
- **Exception**: smelt MAY store operational metadata (schema lineage, DAG, run history)

#### ✅ Rewrite Rules Transform Logical → Physical

- **DO**: Write rewrite rules that are backend-specific
- **DO**: Make rules explicit, testable, and version-controlled
- **DO**: Generate multiple statements from single logical model when needed
- **DON'T**: Put transformation logic in model templates
- **DON'T**: Make analysts think about incrementalization strategies

### Key Differentiators from dbt

| Aspect | dbt | smelt |
|--------|-----|-----|
| Model definition | Jinja templates + SQL | Pure SQL with extensions |
| Logical/physical separation | Mixed in templates | Strict separation via rewrite rules |
| Type checking | None (runtime errors) | Static analysis with LSP support |
| Cross-engine | One target per project | Split work across engines |
| Incrementalization | Macros in models query state | Rewrite rules query backend state |
| Optimization | Manual | Rule-based with semantic analysis |

---

## Target Execution Engines

Initial targets (in priority order):
1. **DuckDB** - Local development, small-medium datasets
2. **PostgreSQL** - Traditional warehouse workloads
3. **Databricks/Spark** - Large-scale distributed processing
4. **DataFusion** - Direct logical plan emission (skip SQL)

Future considerations:
- Flink (streaming)
- Snowflake, BigQuery (cloud warehouses)

---

## Language Design

### Decision: SQL-Based with Extensions

The logical model language is **SQL with smelt-specific extensions**. This choice prioritizes:
- Familiarity for data engineers
- Lower adoption barrier
- Incremental migration from existing SQL

#### Alternatives Considered (Not Chosen)

| Alternative | Pros | Cons | Status |
|-------------|------|------|--------|
| PRQL | Pipeline syntax, less verbose | New syntax to learn, smaller ecosystem | Deferred - could add as frontend later |
| Malloy | Clean semantics, symmetric aggregates | Different execution model, no orchestration | Inspiration only |
| KQL/Kusto | Pipeline syntax, popular for logs | Microsoft-specific heritage | Not pursued |
| Custom DSL | Full control | High investment, adoption friction | Not pursued |

### Extension Syntax: `smelt.*` Functions

Model and metric references use a function-like syntax with the `smelt.` namespace prefix:

```sql
-- Model references
SELECT * FROM smelt.ref('upstream_model')

-- With parameters using => for named arguments (SQL:2003 standard)
SELECT * FROM smelt.ref('upstream_model', filter => event_date > '2024-01-01')

-- Metric references
SELECT
  user_id,
  smelt.metric('monthly_active_users', at => event_date) as mau
FROM events
```

#### Why This Syntax

- **Namespaced**: `smelt.` prefix avoids collision with real UDFs
- **Function-like**: Natural parameter passing with `=>` (standard SQL named parameters)
- **Extensible**: Easy to add `smelt.param()`, `smelt.config()`, etc.
- **Parseable**: Can be handled by extending standard SQL parser

#### Alternatives Considered (Not Chosen)

| Syntax | Example | Reason Not Chosen |
|--------|---------|-------------------|
| Jinja templates | `{{ ref('model') }}` | No static analysis, poor error messages |
| Schema namespace | `smelt.models.upstream` | Less natural for parameters |
| `@` prefix | `@ref('model')` | Potential collision with SQL variables |
| `$` prefix | `$metric.revenue` | Less familiar, edge cases in shells |

### Trailing Commas (smelt extension)

smelt allows trailing commas in SELECT and GROUP BY clauses, matching DuckDB's "friendly SQL" approach:

```sql
SELECT
    user_id,
    order_date,
    amount,  -- trailing comma OK
FROM orders
GROUP BY
    user_id,
    order_date,  -- trailing comma OK
```

This simplifies adding/removing columns and produces cleaner git diffs.

**Supported locations:**
- SELECT column lists
- GROUP BY column lists

**Not supported** (following DuckDB's behavior):
- ORDER BY clauses
- CTEs
- Function arguments

**Industry precedent:**
- DuckDB, BigQuery, Snowflake support trailing commas
- PostgreSQL, MySQL, Oracle, SQL Server, and the SQL standard do not

---

## Two-Layer DSL Architecture

### Layer 1: Metrics DSL (Declarative, Non-SQL)

Captures semantic intent for reusable business metrics. Carries metadata about temporal behavior, statefulness, and decomposability.

```yaml
# Proposed syntax (exact format TBD)
metric monthly_active_users:
  entity: user
  measure: count_distinct(user_id)
  time_grain: day
  period_type: trailing(28 days)
  decomposable: false  # Cannot be computed incrementally per-partition

metric revenue:
  entity: order
  measure: sum(amount)
  dimensions:
    - currency
  decomposable: true  # SUM can be merged across partitions

metric first_touch_attribution:
  entity: user
  event: conversion
  attribute_to: first_in_period(campaign_touch, period: 30 days)
  requires: session_state
```

### Layer 2: SQL Models (Expressive, Familiar)

Use standard SQL with smelt extensions to compose metrics and build complex transformations.

```sql
-- models/daily_user_stats.sql
---
name: daily_user_stats
materialization: table
incremental:
  enabled: true
  event_time_column: event_date
  partition_column: event_date
  granularity: day
---

SELECT
  event_date,
  user_id,
  COUNT(*) as event_count,
  SUM(amount) as daily_revenue
FROM smelt.ref('events')
GROUP BY event_date, user_id
```

### Why Two Layers

1. **Metrics are reusable** - Same definition used across many models
2. **Metrics carry semantics** - Framework knows MAU is trailing-window, revenue is decomposable
3. **SQL stays familiar** - Engineers don't need to learn everything new
4. **Clear optimization boundary** - Metrics heavily optimized, SQL more pass-through

---

## Type System

### Design: Strict with LSP Quick-Fixes

The type system is **strict by default** — this is settled doctrine, not a temporary choice. smelt catches type errors at compile time, not runtime. Cross-family implicit coercion (e.g., `Boolean + Integer`, `Numeric + Varchar`) is always rejected with a clear diagnostic, even though both DuckDB and Spark would accept it. The rationale: implicit coercion is convenient for ad-hoc queries but dangerous in production pipelines where it masks real bugs.

The LSP provides quick-fixes to reduce friction. The goal: committed code is strict, authoring experience is fluid.

For full details on type inference semantics and backend divergences, see [docs/type_semantics.md](type_semantics.md).

```sql
-- User writes:
SELECT a + b FROM t  -- Error: a is VARCHAR, b is INT

-- LSP offers quick-fix, user accepts:
SELECT CAST(a AS INT) + b FROM t  -- Explicit, correct
```

### Key Type System Features

1. **NULL tracking in types**
   - `DECIMAL NOT NULL` vs `DECIMAL NULL`
   - LEFT JOIN automatically promotes to nullable
   - LSP suggests COALESCE when needed

2. **Inference within models, explicit at boundaries**
   - Types inferred for intermediate expressions
   - Input/output schemas must be explicit
   - Similar to Rust: inference in functions, signatures explicit

3. **Superset types with backend validation**
   - The type system can represent types not supported everywhere (e.g., HUGEINT)
   - Error raised only when deploying to a backend that doesn't support it

4. **Literal handling**
   - Numeric literals polymorphic within numeric tower
   - String-to-number coercion always explicit

### SQL Mistakes to Avoid

| SQL Problem | smelt Approach |
|-------------|--------------|
| NULL = NULL is NULL | Require explicit IS NULL checks |
| Implicit type coercion | Require explicit CAST |
| UNION positional matching | UNION BY NAME, error on mismatch |
| SELECT * | Disallow or require explicit opt-in |
| Ambiguous column resolution | Always error, require qualification |
| Non-deterministic GROUP BY (MySQL) | Error on non-aggregated, non-grouped columns |
| ORDER BY in subqueries | Warn or error (meaningless) |
| Implicit CROSS JOIN | Require explicit CROSS JOIN |
| Timestamp timezone ambiguity | Only naive datetime and instant (with tz) |
| Integer division ambiguity | Explicit integer vs decimal division |

---

## Semantic Analysis

### Temporal Dependency Inference

The optimizer analyzes model SQL (via CST inspection) to automatically determine how much historical context each incremental model needs. This drives batch safety decisions and backfill range computation.

Analysis is performed by `smelt-optimizer/src/analysis/temporal.rs`:

```rust
enum TemporalOffset {
    Days(i64),
    Rows(i64),
    Unbounded,
}
```

**Sources detected**: Window functions (`ROWS BETWEEN`, `RANGE`), `LAG`/`LEAD`, JOIN with interval offsets, WHERE with interval patterns.

### Batch Safety Analysis

Based on temporal dependencies, the optimizer classifies each incremental model's batch safety:

| Classification | Meaning | Execution |
|---|---|---|
| `FullyBatchSafe` | No temporal dependencies | Single query for any range |
| `BoundedSafe(n)` | Bounded lookback/lookahead | Auto-sized chunks (3x context, clamped 7-90 days) |
| `PerPartitionOnly` | Unbounded dependencies | Must process one partition at a time |

This drives the `smelt rebuild` command's automatic range expansion and batching strategy.

---

## Dialect Translation

### Design: CST-Walking Printer

The `smelt-dialect` crate provides a dialect-aware printer that walks the Rowan CST in a single forward pass and emits target-specific SQL. This replaces the concept of "rewrite rules" from earlier designs — dialect translation is handled by match arms in the printer, not a separate rule framework.

### Common Rewrites Needed

| Concept | Native Support | Rewrite For Others |
|---------|----------------|-------------------|
| Session windows | Spark, Flink | Window function pattern |
| QUALIFY | DuckDB, Snowflake, Databricks | Subquery with WHERE |
| PIVOT/UNPIVOT | Snowflake, Databricks | CASE expression expansion |
| MERGE/upsert | Most modern engines | DELETE + INSERT |
| Approx count distinct | BigQuery, Spark | HyperLogLog UDF or exact |
| HUGEINT (128-bit) | DuckDB | NUMERIC/DECIMAL elsewhere |
| Recursive CTEs | Postgres, DuckDB, Spark 3.x | Iterative unrolling (limited) |

### Model-Graph Optimization

Separately from dialect translation, the `smelt-optimizer` crate performs model-graph-level optimizations. These read CST structure to detect patterns, then produce `Transformation` instructions (new models, redirected refs, multi-step execution plans). They do **not** mutate existing CSTs.

Current optimization rules:
- **Cube split**: Models with multiple `COUNT(DISTINCT)` aggregations are split into parallel sub-queries joined on GROUP BY keys, reducing memory pressure
- **Incremental materialization**: Models with time-partitioned GROUP BY are detected and converted to incremental DELETE+INSERT patterns
- **Temporal dependency inference**: Window functions, LAG/LEAD, and JOIN intervals are analyzed to determine backfill context requirements

See [architecture_overview.md](architecture_overview.md) for the `Transformation` and `ExecutionStep` enums.

---

## Execution Planning

### ETL Optimization Context

Unlike ad-hoc query optimization, ETL has different constraints:

| Ad-hoc Query | ETL Pipeline |
|--------------|--------------|
| Optimize in ms | Can afford hours of analysis |
| No prior knowledge | Historical run data available |
| Run once | Run daily for years |
| User waiting | Scheduled, unattended |

### Features Enabled by This Context

1. **Pre-run analysis**
   ```bash
   smelt optimize --model daily_stats --budget 4h --sample-data s3://...
   # Outputs learned configuration to .smelt/optimizations/
   ```

2. **Learning from history**
   - Record row counts, shuffle sizes, spill events per run
   - Use historical stats instead of gathering fresh ones
   - Detect patterns (consistent spill → suggest rule)

3. **Human-in-the-loop**
   - Expensive pipelines may warrant manual tuning
   - Framework suggests, engineer confirms

4. **Stored optimization decisions**
   - Persist learned configs across runs
   - Version alongside model definitions

### Batch Processing Intelligence

The framework can prove when batching is safe for backfills:

```sql
-- If model is partition-independent:
--   - All window functions partitioned by batch key
--   - No self-joins across batch boundaries
--   - Aggregations are batch-local or decomposable
-- Then: 90-day backfill = 1 query, not 90 queries
```

Can also transform queries to *make* them batch-safe:

```sql
-- Original (not batch-safe)
ROW_NUMBER() OVER (PARTITION BY user_id ORDER BY ts)

-- Rewritten for batch (if semantics allow)
ROW_NUMBER() OVER (PARTITION BY user_id, batch_date ORDER BY ts)
```

---

## Configuration Layers

### Separation of Concerns

```
┌─────────────────────────────────────────┐
│  Logical Model (analyst)                │
│  - Pure business logic                  │
│  - SQL + smelt.ref/smelt.metric         │
│  - No execution hints                   │
└─────────────────────────────────────────┘
                    │
                    ▼
┌─────────────────────────────────────────┐
│  Execution Config (engineer)            │
│  - Materialization strategy             │
│  - Backend hints                        │
│  - Optimization budget                  │
└─────────────────────────────────────────┘
                    │
                    ▼
┌─────────────────────────────────────────┐
│  Learned Optimizations (framework)      │
│  - Historical statistics                │
│  - Successful rule applications         │
│  - Performance baselines                │
└─────────────────────────────────────────┘
```

### Configuration Syntax

> **Status**: YAML frontmatter in `.sql` files is the implemented config surface, with `smelt.yml` for project-level overrides. The annotation syntax (`@materialize`, `@partition_by`) shown below was an early design option that has not been implemented. It may be revisited in the future.

**Current approach: YAML frontmatter**
```sql
---
name: daily_stats
materialization: table
incremental:
  enabled: true
  event_time_column: event_date
  partition_column: event_date
  granularity: day
---
SELECT ...
```

**Previously proposed: Annotations in SQL comments** *(not implemented)*
```sql
-- @materialize: daily
-- @partition_by: event_date
-- @backend_hint(spark): { coalesce_partitions: 200 }
SELECT ...
```

**Previously proposed: Separate config file** *(smelt.yml serves this role)*
```yaml
# smelt.yml
models:
  daily_stats:
    materialization: table
    incremental:
      enabled: true
      event_time_column: event_date
      partition_column: event_date
      granularity: day
```

---

## State Management for Computations

### Design: Framework Does NOT Manage State

smelt generates artifacts for target engines to manage state natively. It does NOT implement its own state storage.

| Pattern | Databricks/Spark | Flink | Postgres |
|---------|------------------|-------|----------|
| Incremental | MERGE with partition overwrite | Changelog stream | UPSERT |
| Running totals | Batch recompute or Delta | Managed state + checkpoints | Materialized view |
| Sessions | session_window() | Session windows | Window function rewrite |

### Framework Responsibilities

1. **Validate** - Target engine supports required semantics
2. **Generate** - Correct artifacts for target's state model
3. **Error clearly** - "Model X requires session semantics, postgres_batch doesn't support this"

### Migration Path

If a model is deployed to Spark batch today and moves to Flink streaming tomorrow:
- Logical model unchanged
- Execution config changes target
- Framework generates new artifacts

---

## LSP and Developer Experience

### Quick-Fix Driven Strictness

```
┌────────────────────────────────────────────────────────────┐
│  SELECT a + b FROM t                                       │
│          ~~~                                               │
│  Error: Cannot add VARCHAR and INT                         │
│                                                            │
│  Quick fixes:                                              │
│    • Cast 'a' to INT: CAST(a AS INT) + b                  │
│    • Cast 'b' to VARCHAR: a + CAST(b AS VARCHAR)          │
└────────────────────────────────────────────────────────────┘
```

### LSP Features

- **Autocomplete**: Model names, metric names, column names from upstream
- **Hover**: Show inferred types, metric definitions, upstream schema
- **Go to definition**: Jump to model/metric definition
- **Find references**: Where is this model/metric used?
- **Diagnostics**: Type errors, unknown references, deprecated features
- **Quick fixes**: Add casts, qualify ambiguous columns, add COALESCE

### Error Quality

```
Error: Model 'daily_stats' cannot be deployed to 'duckdb_batch'

Reason: Model requires 'Sessionized' computation (line 15: session_window(...))
        but 'duckdb_batch' does not support native sessions.

Options:
  1. Deploy to 'spark_streaming' (supports sessions natively)
  2. Add '@allow_complex_rewrite' to enable window-function emulation
  3. Refactor model to remove session dependency
```

---

## Comparison with Related Tools

### vs dbt

| Aspect | dbt | smelt |
|--------|-----|-----|
| Model definition | Jinja + SQL templates | Parsed SQL with extensions |
| Ref resolution | Runtime template expansion | Static analysis |
| Type safety | None | Full type system |
| Incrementalization | Manual `is_incremental()` | Automatic semantic analysis |
| Backfill batching | Run N times | Prove safety, run once |
| Cross-engine | No | Yes |
| Optimization | Manual | Rule-based + learning |

### vs Malloy

| Aspect | Malloy | smelt |
|--------|--------|-----|
| Primary user | Analyst exploring data | Engineer building pipelines |
| Execution | Query-time SQL generation | Planned materialization |
| Orchestration | None | Built-in |
| Cross-engine | Single target | Can split across engines |
| Incrementalization | Not in scope | Core feature |
| State management | None | Via target engine |

Malloy is a better query language for analysts. smelt is infrastructure for data engineers.

### vs Substrait

Substrait defines a cross-engine plan representation. smelt could potentially:
- Use Substrait as an IR layer
- Emit Substrait plans for DataFusion backend
- Benefit from Substrait's type system work

### vs Apache Calcite

Calcite is a query optimizer framework. smelt differs:
- Calcite optimizes single queries; smelt optimizes pipeline DAGs
- Calcite focuses on join ordering; smelt focuses on materialization/incrementalization
- smelt delegates low-level optimization to target engines

---

## Incremental Table Builds

### Philosophy: Separation of Logical and Physical

Incremental table builds are implemented through **rewrite rules**, not macros or framework magic.

**Three clear layers:**

1. **Logical Model** (Analyst writes)
   - Pure SQL expressing business intent
   - No conditionals, no templating, no physical concerns
   - Just `SELECT` with `smelt.ref()` and `smelt.metric()`

2. **Rewrite Rules** (Engineer writes)
   - Transform logical model → backend-specific physical SQL
   - Handle incrementalization strategies per backend
   - Explicit, testable, version-controlled transformations

3. **Backend Execution** (Engine manages)
   - Delta Lake: Transaction log tracks partitions
   - Flink: Checkpoints track stream position
   - DuckDB: Table state and transaction history
   - **Backends own computational state** (watermarks, offsets)

Analyst writes pure business logic:

```sql
-- models/daily_revenue.sql
---
name: daily_revenue
materialization: table
incremental:
  enabled: true
  event_time_column: order_date
  partition_column: order_date
  granularity: day
---

SELECT
  order_date,
  customer_id,
  SUM(amount) as total
FROM smelt.ref('orders')
GROUP BY order_date, customer_id
```

**No conditionals. No templating. Just what to compute.**

### Example: Backend-Specific Execution

The optimizer and backend crates generate backend-specific SQL from the pure logical model:

#### DuckDB (DELETE + INSERT)

```sql
-- Generated by smelt for incremental run --start 2026-03-20 --end 2026-03-25

-- Step 1: Delete affected partitions
DELETE FROM daily_revenue
WHERE order_date >= DATE '2026-03-20' AND order_date < DATE '2026-03-25';

-- Step 2: Insert fresh data
INSERT INTO daily_revenue
SELECT order_date, customer_id, SUM(amount) as total
FROM orders
WHERE order_date >= DATE '2026-03-20' AND order_date < DATE '2026-03-25'
GROUP BY order_date, customer_id;
```

#### Databricks/Delta Lake (MERGE)

```sql
-- Future: Spark backend will generate MERGE for incremental
MERGE INTO daily_revenue AS target
USING (
    SELECT order_date, customer_id, SUM(amount) as total
    FROM orders
    WHERE order_date >= DATE '2026-03-20' AND order_date < DATE '2026-03-25'
    GROUP BY order_date, customer_id
) AS source
ON target.order_date = source.order_date
   AND target.customer_id = source.customer_id
WHEN MATCHED THEN UPDATE SET *
WHEN NOT MATCHED THEN INSERT *
```

The logical model is identical regardless of backend — the framework generates the right execution strategy.

### What Each Layer Owns

| Responsibility | Owner | Examples |
|----------------|-------|----------|
| **Business logic** | Logical model | What aggregations, joins, filters |
| **Incrementalization strategy** | Rewrite rules | MERGE vs DELETE+INSERT vs streaming |
| **Computational state** | Backend engine | Watermarks, stream offsets, transaction logs |
| **Schema lineage** | smelt metadata | How table was derived, what changed |
| **Execution orchestration** | smelt framework | DAG order, parallelization, retries |
| **Semantic analysis** | smelt framework | Detect unsafe patterns, suggest optimizations |

### Configuration

Models declare metadata through YAML frontmatter or `smelt.yml`. Temporal dependencies (how much historical context each model needs) are inferred automatically from the SQL via AST analysis. Per-column `data_latency` on upstream sources can be specified for late-arriving data. See [docs/plans/20260322-incremental-model-support.md](plans/20260322-incremental-model-support.md).

```sql
-- models/daily_revenue.sql
---
name: daily_revenue
materialization: table
incremental:
  enabled: true
  event_time_column: order_date
  partition_column: order_date
  granularity: day
---

SELECT
  order_date,
  customer_id,
  SUM(amount) as total
FROM smelt.ref('orders')
GROUP BY order_date, customer_id
```

Or in project config:
```yaml
# smelt.yml
models:
  daily_revenue:
    materialization: table
    incremental:
      enabled: true
      event_time_column: order_date
      partition_column: order_date
      granularity: day
```

**Configuration tells rewrite rules HOW to transform, but doesn't change WHAT is computed.**

### What NOT to Do

#### ❌ NO MACROS in Logical Models

**Don't do this** (dbt-style macros):
```sql
-- ❌ WRONG - Macros pollute logical models
{% if is_incremental() %}
  DELETE FROM {{ this }} WHERE date >= {{ var('start_date') }}
{% endif %}

SELECT * FROM source
```

**Do this instead**:
```sql
-- ✅ CORRECT - Pure logical model
SELECT * FROM smelt.ref('source')
```

```yaml
# ✅ CORRECT - YAML frontmatter configures incrementalization
---
incremental:
  enabled: true
  event_time_column: event_date
  partition_column: event_date
  granularity: day
---
```

**Why**:
- Logical models should express business logic, not execution strategy
- Macros make models harder to analyze, optimize, and understand
- The optimizer and backend handle incrementalization automatically

#### ❌ NO COMPUTATIONAL STATE in smelt

**Don't do this**:
```python
# ❌ WRONG - smelt tracking watermarks
smelt_state = {
    'daily_revenue': {
        'watermark': '2024-01-17',
        'last_offset': 12345
    }
}
```

**Do this instead**:
```sql
-- ✅ CORRECT - Delta Lake tracks state
MERGE INTO daily_revenue ...
-- Delta's transaction log knows what's been written

-- ✅ CORRECT - Flink tracks state
INSERT INTO daily_revenue ...
-- Flink checkpoints track stream position

-- ✅ CORRECT - Query backend for state
SELECT MAX(order_date) FROM daily_revenue
-- Let DuckDB tell us what exists
```

**Why**:
- Backends are designed to manage computational state (checkpoints, transaction logs)
- Duplicating state creates consistency problems
- smelt is a compiler/orchestrator, not a runtime execution engine

**smelt MAY store operational metadata** (not computational state):
- ✅ Schema lineage: How was this table derived?
- ✅ DAG dependencies: What models depend on what?
- ✅ Run history: Performance metrics, row counts, timestamps
- ✅ Deployed versions: What version of model is running?

But NOT:
- ❌ Watermarks (what data has been processed)
- ❌ Stream offsets (where in the stream we are)
- ❌ Batch boundaries (what batches are pending)

### Semantic Analysis

smelt analyzes logical models to detect unsafe patterns:

```
$ smelt run --backend delta

Analyzing daily_revenue...
  ✓  Model is partition-independent (safe for incremental)
  ✓  Time column 'order_date' found in GROUP BY
  ✓  No cross-partition window functions

Applying rewrite rule: incremental_merge_delta
Generated MERGE statement for Delta Lake

user_sessions:
  ⚠️  Warning: Window function ROW_NUMBER() OVER (PARTITION BY user_id ...)
      crosses batch boundaries. Incremental may produce incorrect results.

      Options:
        1. Add user_id to partition_by (make it batch-local)
        2. Force full refresh for this model
        3. Use lookback to capture all user history

Proceed? [Y/n]
```

### CLI Interface

```bash
# Deploy models (uses backend's incremental strategy)
smelt run --backend delta

# Full refresh specific model
smelt run --backend delta --full-refresh --select daily_revenue

# Dry run (show generated SQL without executing)
smelt run --backend delta --dry-run

# Show what rewrite rules will be applied
smelt explain daily_revenue --backend delta
```

### Comparison with dbt

**dbt approach** (macros in logical models):
```sql
{{ config(materialized='incremental') }}

SELECT order_date, customer_id, SUM(amount)
FROM {{ source('raw', 'orders') }}
{% if is_incremental() %}
WHERE order_date >= '{{ var("start_date") }}'
{% endif %}
GROUP BY 1, 2
```

**smelt approach** (config separate from logic):
```sql
-- Logical model (pure) with YAML frontmatter config
---
incremental:
  enabled: true
  event_time_column: order_date
  partition_column: order_date
  granularity: day
---
SELECT order_date, customer_id, SUM(amount)
FROM smelt.ref('orders')
GROUP BY order_date, customer_id
-- The optimizer and backend generate incremental SQL automatically
```

**Key differences:**

| Aspect | dbt | smelt |
|--------|-----|-------|
| **Logical models** | Mixed with execution logic | Pure business logic |
| **Incrementalization** | User writes conditionals | Rewrite rules generate |
| **Backend-specific** | Manual per backend | Rules per backend |
| **Analysis** | Limited (opaque templates) | Full (parsed semantics) |
| **Transparency** | Hidden in macros | Explicit in rules |
| **Customization** | Edit model templates | Write custom rules |

smelt separates "what to compute" from "how to execute" more strictly than dbt.

### Schema Lineage Tracking

**Problem**: When a model changes, we need to know how to efficiently migrate the table.

**smelt tracks schema lineage** - not just current schema, but how it was derived:

```yaml
# .smelt/lineage/daily_revenue.yaml
model: daily_revenue
current_version: 3
deployed_schema:
  columns:
    - name: order_date
      type: DATE
      derived_from: source.orders.order_date
    - name: customer_id
      type: INTEGER
      derived_from: source.orders.customer_id
    - name: total
      type: DECIMAL(10,2)
      expression: SUM(amount)
      depends_on: [source.orders.amount]

history:
  - version: 2
    deployed_at: 2024-01-10
    changes:
      - added column 'total'
      - expression: SUM(amount)
      - backfill: computed from existing orders table
```

**Why track lineage**:

1. **Efficient schema evolution**
   ```sql
   -- Model adds: revenue_per_customer = total / customer_count

   -- smelt knows 'total' is already in the table
   -- Can compute new column without re-reading source:
   ALTER TABLE daily_revenue ADD COLUMN revenue_per_customer DECIMAL(10,2);
   UPDATE daily_revenue
   SET revenue_per_customer = total / customer_count;
   ```

2. **Incremental column backfill**
   ```sql
   -- Model adds column from upstream model
   -- smelt knows customers table is already materialized
   -- Can join to backfill:
   ALTER TABLE daily_revenue ADD COLUMN customer_tier VARCHAR;
   UPDATE daily_revenue d
   SET customer_tier = (
     SELECT tier FROM customers c WHERE c.id = d.customer_id
   );
   ```

3. **Optimization suggestions**
   ```
   $ smelt run

   Schema change detected in daily_revenue:
     + shipping_cost (from orders.shipping_cost)

   Options:
     1. Full refresh (safe, slow: recompute all 10B rows)
     2. Incremental backfill (fast: only recent partitions from source)
     3. Compute from existing data (fastest: if computable from 'total')

   Recommendation: Option 2 (incremental backfill last 90 days)
   ```

**Schema lineage is metadata, not computational state**:
- ✅ Tracks: How was this derived? What depends on what?
- ❌ Does NOT track: What data has been processed? (that's backend state)

---

## Schema Evolution

When model definitions change, smelt can efficiently update existing materialized tables instead of requiring full rebuilds.

### The Problem

In dbt, any schema change requires a full refresh:
```sql
-- Before: SELECT a, b FROM source
-- After:  SELECT a, b, c FROM source

-- dbt approach: DROP TABLE and rebuild from scratch
-- Even if the table has 10 billion rows and 'c' is cheap to compute
```

### smelt's Approach

Because smelt tracks schemas and understands SQL semantics, it can generate efficient migrations:

```sql
-- Adding a column
ALTER TABLE daily_revenue ADD COLUMN new_metric DECIMAL;
UPDATE daily_revenue SET new_metric = (
  SELECT SUM(amount) FROM orders WHERE orders.date = daily_revenue.date
);

-- Or for additive columns with defaults
ALTER TABLE daily_revenue ADD COLUMN region VARCHAR DEFAULT 'unknown';
```

### Change Detection

smelt compares the current model definition against the last-deployed schema:

```
$ smelt run

Schema changes detected:

daily_revenue:
  + new_metric DECIMAL     (added column)
  ~ amount DECIMAL(10,2)   (was: DECIMAL - precision change)
  - old_column             (removed column)

Migration strategy:
  • new_metric: ALTER TABLE ADD COLUMN + backfill query
  • amount: Safe widening, no action needed
  • old_column: Will be dropped (data loss)

Proceed? [Y/n]
```

### Evolution Strategies

| Change Type | Strategy | Data Preserved? |
|-------------|----------|-----------------|
| Add column (computable) | ALTER + UPDATE | ✅ Yes |
| Add column (with default) | ALTER + DEFAULT | ✅ Yes |
| Add column (needs source) | Full refresh | ✅ Yes |
| Remove column | ALTER DROP | ⚠️ Column lost |
| Widen type (INT→BIGINT) | No action | ✅ Yes |
| Narrow type (BIGINT→INT) | Validate + ALTER | ⚠️ May fail |
| Change type (incompatible) | Full refresh | ✅ Yes |
| Rename column | ALTER RENAME | ✅ Yes |

### Efficient Backfill for New Columns

When adding a column, smelt analyzes whether it can be computed from existing data:

**Case 1: Column derived from existing columns**
```sql
-- Model adds: total_with_tax AS amount * 1.1
-- smelt generates:
ALTER TABLE orders ADD COLUMN total_with_tax DECIMAL;
UPDATE orders SET total_with_tax = amount * 1.1;
```

**Case 2: Column from upstream model (already materialized)**
```sql
-- Model adds: customer_name from smelt.ref('customers')
-- smelt generates:
ALTER TABLE orders ADD COLUMN customer_name VARCHAR;
UPDATE orders o SET customer_name = (
  SELECT c.name FROM customers c WHERE c.id = o.customer_id
);
```

**Case 3: Column requires source data**
```sql
-- Model adds: new_field from smelt.ref('raw_events')
-- If raw_events is a source (not materialized), full refresh needed
-- smelt warns and offers options:
--   1. Full refresh (safe, slow)
--   2. Set to NULL/default for existing rows (fast, incomplete)
--   3. Incremental backfill over time windows
```

### Cross-Model Evolution

When a model's schema changes, smelt analyzes downstream impact:

```
$ smelt run

Schema change in 'orders':
  + shipping_cost DECIMAL

Downstream impact analysis:

  daily_revenue (depends on orders):
    • No impact - doesn't select shipping_cost

  order_summary (depends on orders):
    • Uses SELECT * - will automatically include new column
    • Downstream schema will change
    • Cascade: customer_report also uses SELECT *

Options:
  1. Update all downstream models (recommended)
  2. Update only direct dependents
  3. Update orders only (downstream will fail on next run)
```

### Configuration

Control evolution behavior per-model or globally:

```yaml
# smelt.yml
schema_evolution:
  strategy: prompt           # prompt, auto, strict
  allow_column_removal: true
  allow_type_narrowing: false

models:
  critical_table:
    schema_evolution:
      strategy: strict       # Never auto-migrate, always prompt
      allow_column_removal: false
```

Or via annotations:
```sql
-- @schema_evolution: strict
-- @schema_evolution.allow_column_removal: false

SELECT ...
```

### CLI Commands

```bash
# Show pending schema changes without applying
smelt diff

# Apply schema migrations
smelt run --migrate

# Force full refresh even when migration is possible
smelt run --full-refresh

# Generate migration SQL without executing
smelt migrate --dry-run --output migrations/

# Validate that schema changes are safe
smelt validate
```

### State Tracking

smelt tracks deployed schemas:

```yaml
# .smelt/state/daily_revenue.state.yaml
model: daily_revenue
schema:
  version: 3
  deployed_at: 2024-01-18T06:00:00Z
  columns:
    - name: order_date
      type: DATE
      nullable: false
    - name: customer_id
      type: INTEGER
      nullable: false
    - name: total
      type: DECIMAL(10,2)
      nullable: true
  history:
    - version: 2
      deployed_at: 2024-01-10T06:00:00Z
      changes: ["added column: total"]
    - version: 1
      deployed_at: 2024-01-01T06:00:00Z
      changes: ["initial deployment"]
```

### Integration with Incremental

Schema evolution works with incremental builds:

```
Scenario: Add new column to incremental model

1. smelt detects schema change (new column added)
2. For existing rows: ALTER TABLE + backfill UPDATE
3. For new rows: Normal incremental INSERT includes new column
4. Result: Complete data, minimal recomputation
```

```sql
-- Combined migration + incremental
BEGIN TRANSACTION;

-- Schema migration
ALTER TABLE daily_revenue ADD COLUMN new_metric DECIMAL;
UPDATE daily_revenue SET new_metric = compute_metric(...)
WHERE TRUE;  -- All existing rows

-- Incremental update (new data)
DELETE FROM daily_revenue WHERE order_date >= '2024-01-18';
INSERT INTO daily_revenue
SELECT order_date, customer_id, total, compute_metric(...) as new_metric
FROM orders
WHERE order_date >= '2024-01-18';

COMMIT;
```

---

## Implementation Phases

### Phase 1: Core Parser and Single Backend
- SQL parser with `smelt.ref()` extension
- Basic type checking
- DuckDB backend
- Simple model dependencies
- No incrementalization

### Phase 2: Type System and LSP
- Full type inference
- NULL tracking
- LSP with diagnostics and quick-fixes
- Multiple models, dependency resolution

### Phase 3: Multi-Backend and Rewrites
- Add Postgres, Spark backends
- Rewrite rule framework
- Backend capability declarations
- Basic rule library (QUALIFY, PIVOT, etc.)

### Phase 4: Metrics DSL
- Metric definition syntax
- `smelt.metric()` resolution
- Temporal semantics metadata
- Metric composition

### Phase 5: Incrementalization
- Computation requirement analysis
- Batch safety proofs
- Incremental rewrite rules
- State requirement validation

### Phase 6: Learning and Optimization
- Run history capture
- Statistics persistence
- Optimization budget system
- Recommendation engine

---

## Open Questions

1. **Metrics DSL syntax**: YAML? Custom DSL? SQL-like?

2. **Substrait integration**: Use as IR layer? Just for DataFusion?

3. **Testing strategy**: How to verify dialect translation correctness across engines? (Property tests cover type inference; dialect output correctness is less systematic.)

4. **Lineage/Catalog integration**: How to expose to external catalogs?

5. **Secrets/connections**: How to configure database connections?

---

## Appendix: SQL Extension Grammar (Sketch)

```ebnf
smelt_ref ::= 'smelt.ref' '(' string_literal (',' ref_param)* ')'
ref_param ::= identifier '=>' expr

smelt_metric ::= 'smelt.metric' '(' string_literal (',' metric_param)* ')'
metric_param ::= identifier '=>' expr

-- smelt functions can appear in:
--   FROM clause (smelt.ref)
--   SELECT expressions (smelt.metric)
--   WHERE/HAVING (smelt.metric for filtering)
```

---

## Appendix: Example End-to-End

**Model definition:**
```sql
-- models/daily_revenue.sql
---
name: daily_revenue
materialization: table
incremental:
  enabled: true
  event_time_column: order_date
  partition_column: order_date
  granularity: day
---

SELECT
  order_date,
  customer_id,
  SUM(amount) as daily_revenue
FROM smelt.ref('orders')
GROUP BY order_date, customer_id
```

**Project config:**
```yaml
# smelt.yml
name: my_project
models_dir: models
backend: duckdb
```

**Run:**
```bash
smelt run --start 2026-03-01 --end 2026-03-25
# Framework:
#   1. Parses model, resolves refs
#   2. Analyzes temporal dependencies
#   3. Determines batch safety
#   4. Generates dialect-specific SQL
#   5. Creates incremental merge logic
#   6. Outputs to configured location
```
