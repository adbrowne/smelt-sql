# sqt
Semantic Query Tool

Andrew's Christmas 2025 holiday project.

# sqt (Semantic Query Transformation) - Design Specification

## Overview

sqt is a data transformation framework that separates **logical transformation definitions** from **physical execution planning**. Unlike traditional tools like dbt that use SQL templates, sqt parses and understands the semantics of transformations, enabling advanced capabilities like automatic refactoring, cross-engine deployment, and intelligent incrementalization.

### Core Philosophy

1. **Analysts define what** - Pure logical models expressing business intent
2. **Engineers define how** - Rewrite rules and execution configuration
3. **Framework mediates** - Validates, optimizes, and deploys to target engines

### Key Differentiators from dbt

| Aspect | dbt | sqt |
|--------|-----|-----|
| Model definition | Jinja templates | Parsed semantic models |
| Type checking | None (runtime errors) | Static analysis with LSP support |
| Cross-engine | One target per project | Split work across engines |
| Optimization | Manual | Rule-based with learning |
| Incrementalization | Manual `is_incremental()` | Semantic analysis of what's safe |

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

The logical model language is **SQL with sqt-specific extensions**. This choice prioritizes:
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

### Extension Syntax: `sqt.*` Functions

Model and metric references use a function-like syntax with the `sqt.` namespace prefix:

```sql
-- Model references
SELECT * FROM sqt.ref('upstream_model')

-- With parameters using => for named arguments (SQL:2003 standard)
SELECT * FROM sqt.ref('upstream_model', filter => event_date > '2024-01-01')

-- Metric references
SELECT 
  user_id,
  sqt.metric('monthly_active_users', at => event_date) as mau
FROM events
```

#### Why This Syntax

- **Namespaced**: `sqt.` prefix avoids collision with real UDFs
- **Function-like**: Natural parameter passing with `=>` (standard SQL named parameters)
- **Extensible**: Easy to add `sqt.param()`, `sqt.config()`, etc.
- **Parseable**: Can be handled by extending standard SQL parser

#### Alternatives Considered (Not Chosen)

| Syntax | Example | Reason Not Chosen |
|--------|---------|-------------------|
| Jinja templates | `{{ ref('model') }}` | No static analysis, poor error messages |
| Schema namespace | `sqt.models.upstream` | Less natural for parameters |
| `@` prefix | `@ref('model')` | Potential collision with SQL variables |
| `$` prefix | `$metric.revenue` | Less familiar, edge cases in shells |

### Reference Parameters

```sql
sqt.ref('model',
  -- Data filtering
  filter => <expr>,              -- Pushdown predicate
  partitions => ['2024-01'],     -- Explicit partition list
  sample => 0.1,                 -- Sampling ratio
  
  -- Freshness/versioning  
  max_staleness => '1 hour',     -- Acceptable data age
  as_of => '2024-01-01',         -- Time travel
  version => 'v2',               -- Explicit model version
  
  -- Optimizer hints
  prefer_materialized => true,   -- Use cache if available
  allow_approximate => true,     -- OK to use approximations
  inline => true                 -- Don't materialize, inline SQL
)

sqt.metric('metric_name',
  at => event_date,              -- Point-in-time evaluation
  for => user_id,                -- Entity to compute for
  grain => 'daily',              -- Rollup level
  allow_approximate => true      -- HLL acceptable
)
```

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

Use standard SQL with sqt extensions to compose metrics and build complex transformations.

```sql
-- Model: daily_user_stats
-- @materialize: daily
-- @partition_by: event_date

SELECT 
  event_date,
  user_id,
  sqt.metric('revenue', at => event_date) as daily_revenue,
  sqt.metric('monthly_active_users', at => event_date) as mau_at_date
FROM sqt.ref('events')
GROUP BY 1, 2
```

### Why Two Layers

1. **Metrics are reusable** - Same definition used across many models
2. **Metrics carry semantics** - Framework knows MAU is trailing-window, revenue is decomposable
3. **SQL stays familiar** - Engineers don't need to learn everything new
4. **Clear optimization boundary** - Metrics heavily optimized, SQL more pass-through

---

## Type System

### Design: Strict with LSP Quick-Fixes

The type system is strict (inspired by Haskell) but the LSP provides quick-fixes to reduce friction. The goal: committed code is strict, authoring experience is fluid.

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
   - IR can represent types not supported everywhere (e.g., HUGEINT)
   - Error raised only when deploying to a backend that doesn't support it

4. **Literal handling**
   - Numeric literals polymorphic within numeric tower
   - String-to-number coercion always explicit

### SQL Mistakes to Avoid

| SQL Problem | sqt Approach |
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

## Intermediate Representation (IR)

### Computation Requirements

The IR tracks what each computation *requires* semantically, not how to execute it:

```rust
enum ComputationRequirement {
    Stateless,           // Pure function of current row
    PartitionLocal,      // Independent per partition key
    RequiresOrdering,    // Needs rows in specific order within partition
    Windowed {           // Needs N prior/future rows
        preceding: WindowBound,
        following: WindowBound,
    },
    Sessionized {        // Gap-based grouping
        key: Vec<Column>,
        gap: Duration,
    },
    Cumulative,          // Depends on all prior rows
}
```

### Backend Capability Declaration

Each backend declares what it supports:

```rust
struct BackendCapabilities {
    supports_stateless: bool,
    supports_partition_local: bool,
    supports_ordering: bool,
    supports_windowed: bool,
    supports_sessionized: bool,      // Spark: native, DuckDB: via rewrite
    supports_cumulative: bool,
    supports_types: HashSet<DataType>,
    // ... engine-specific capabilities
}
```

### Validation Flow

1. Parse SQL + extensions → AST
2. Resolve references (models, metrics) → Typed AST
3. Analyze computation requirements → Annotated IR
4. Match against target backend → Error or rewrite plan

---

## Rewrite Rules

### Design: Engine-Specific Translations

Rewrite rules translate semantic concepts to engine-specific implementations. They live in the framework, not in user models.

```python
# Example: Sessionization
@rule
def sessionization_spark(node: SessionizedComputation, target: SparkBackend):
    """Native session_window in Spark"""
    return SparkSessionWindow(
        keys=node.keys,
        gap=node.gap,
        timestamp_col=node.timestamp
    )

@rule(complexity="high")
def sessionization_duckdb(node: SessionizedComputation, target: DuckDBBackend):
    """Implement via window functions"""
    return WindowBasedSessionization(
        flag_expr=f"""
            CASE WHEN {node.timestamp} - LAG({node.timestamp}) OVER (
                PARTITION BY {', '.join(node.keys)} ORDER BY {node.timestamp}
            ) > INTERVAL '{node.gap}'
            THEN 1 ELSE 0 END
        """,
        session_id_expr="SUM(flag) OVER (PARTITION BY ... ORDER BY ...)"
    )
```

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

### Prior Art for Rewrite Systems

| System | Relevance | Key Ideas |
|--------|-----------|-----------|
| **Egg** (e-graphs) | Rule framework | Equality saturation, avoid ordering issues |
| **MLIR** | Multi-level IR | Progressive lowering, dialects |
| **Apache Calcite** | Query optimization | RelOptRule, cost-based selection |
| **DataFusion optimizer** | Rust-native | Simple OptimizerRule trait |
| **Substrait** | Cross-engine IR | Portable plan representation |

### Rule Interface (Proposed)

Rust core with Python bindings for rule authoring:

```python
@rule
def my_rule(node: CountDistinct, ctx: RuleContext) -> Optional[Rewrite]:
    if ctx.target.supports(ApproxCountDistinct) and ctx.has_annotation('approximate_ok'):
        return HyperLogLog(node.column, precision=14)
    return None  # No rewrite, use default
```

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
   sqt optimize --model daily_stats --budget 4h --sample-data s3://...
   # Outputs learned configuration to .sqt/optimizations/
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
│  - SQL + sqt.ref/sqt.metric             │
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

### Configuration Syntax (Proposed)

**Option A: Annotations in SQL comments**
```sql
-- @materialize: daily
-- @partition_by: event_date
-- @backend_hint(spark): { coalesce_partitions: 200 }
SELECT ...
```

**Option B: Separate config file**
```yaml
# daily_stats.config.yaml
model: daily_stats
materialize:
  grain: daily
  partition_by: event_date
backend_hints:
  spark:
    coalesce_partitions: 200
    avoid_cube: true
optimization:
  budget: 2h
  learn_from_history: true
```

**Option C: Both** (annotations for simple, file for complex)

Recommendation: Start with Option A for simplicity, add Option B when needed.

---

## State Management for Computations

### Design: Framework Does NOT Manage State

sqt generates artifacts for target engines to manage state natively. It does NOT implement its own state storage.

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

| Aspect | dbt | sqt |
|--------|-----|-----|
| Model definition | Jinja + SQL templates | Parsed SQL with extensions |
| Ref resolution | Runtime template expansion | Static analysis |
| Type safety | None | Full type system |
| Incrementalization | Manual `is_incremental()` | Automatic semantic analysis |
| Backfill batching | Run N times | Prove safety, run once |
| Cross-engine | No | Yes |
| Optimization | Manual | Rule-based + learning |

### vs Malloy

| Aspect | Malloy | sqt |
|--------|--------|-----|
| Primary user | Analyst exploring data | Engineer building pipelines |
| Execution | Query-time SQL generation | Planned materialization |
| Orchestration | None | Built-in |
| Cross-engine | Single target | Can split across engines |
| Incrementalization | Not in scope | Core feature |
| State management | None | Via target engine |

Malloy is a better query language for analysts. sqt is infrastructure for data engineers.

### vs Substrait

Substrait defines a cross-engine plan representation. sqt could potentially:
- Use Substrait as an IR layer
- Emit Substrait plans for DataFusion backend
- Benefit from Substrait's type system work

### vs Apache Calcite

Calcite is a query optimizer framework. sqt differs:
- Calcite optimizes single queries; sqt optimizes pipeline DAGs
- Calcite focuses on join ordering; sqt focuses on materialization/incrementalization
- sqt delegates low-level optimization to target engines

---

## Implementation Phases

### Phase 1: Core Parser and Single Backend
- SQL parser with `sqt.ref()` extension
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
- `sqt.metric()` resolution
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

2. **Config location**: Annotations, separate files, or both?

3. **Rule language**: Pure Rust? Python bindings? Declarative DSL?

4. **Substrait integration**: Use as IR layer? Just for DataFusion?

5. **Testing strategy**: How to verify rewrite correctness across engines?

6. **Lineage/Catalog integration**: How to expose to external catalogs?

7. **Secrets/connections**: How to configure database connections?

---

## Appendix: SQL Extension Grammar (Sketch)

```ebnf
sqt_ref ::= 'sqt.ref' '(' string_literal (',' ref_param)* ')'
ref_param ::= identifier '=>' expr

sqt_metric ::= 'sqt.metric' '(' string_literal (',' metric_param)* ')'
metric_param ::= identifier '=>' expr

-- sqt functions can appear in:
--   FROM clause (sqt.ref)
--   SELECT expressions (sqt.metric)
--   WHERE/HAVING (sqt.metric for filtering)
```

---

## Appendix: Example End-to-End

**Metric definition:**
```yaml
metric revenue:
  measure: sum(amount)
  entity: order
  decomposable: true
```

**Model definition:**
```sql
-- models/daily_revenue.sql
-- @materialize: daily
-- @partition_by: order_date

SELECT 
  order_date,
  customer_id,
  sqt.metric('revenue') as daily_revenue
FROM sqt.ref('orders', filter => order_date >= current_date - 90)
GROUP BY 1, 2
```

**Execution config:**
```yaml
# execution/daily_revenue.yaml
model: daily_revenue
targets:
  - name: dev
    backend: duckdb
  - name: prod
    backend: spark
    hints:
      coalesce_partitions: 100
```

**Deploy:**
```bash
sqt deploy --model daily_revenue --target prod
# Framework:
#   1. Parses model, resolves metric
#   2. Checks Spark supports all requirements
#   3. Applies rewrites if needed
#   4. Generates Spark SQL
#   5. Creates incremental merge logic
#   6. Outputs to configured location
```
