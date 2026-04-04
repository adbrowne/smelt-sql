# Architecture

smelt is a **SQL-to-SQL compiler and orchestrator** for data pipelines. It parses SQL models written in smelt's dialect, resolves dependencies, optionally optimizes across model boundaries, and emits dialect-specific SQL for target execution engines.

Three ideas make smelt different from dbt:

1. **Logical/Physical Separation** -- Users write WHAT to compute; the planner and dialect-aware printer decide HOW it executes on each backend.
2. **Cross-Model Planning** -- The planner operates at the model-graph level (creating shared materializations, redirecting refs, merging models), not at the expression level.
3. **First-Class Editor Support** -- LSP + Salsa + Rowan for incremental compilation and real-time diagnostics.

---

## Compilation Pipeline

Source files flow through five stages before reaching a target engine:

```
Source Files (.sql / .py)
     |
     v
Parse  (smelt-parser)       -- Rowan CST, error-recovery, lossless
     |
     v
Analyze  (smelt-db)         -- Salsa incremental queries: refs, types, diagnostics
     |
     v
Plan  (smelt-planner)       -- Model-graph transforms (optional)
     |
     v
Generate  (smelt-dialect)   -- CST walk --> target-specific SQL string
     |
     v
Execute  (smelt-backend-*)  -- Send SQL to DuckDB / Spark / etc.
```

**Key invariant**: The Rowan CST is the single representation from parse through generation. There is no intermediate IR. This avoids fidelity boundaries and preserves comments, formatting, and smelt extensions throughout the pipeline.

### Stage Details

**Parse** -- The parser uses recursive descent with error recovery at sync points (semicolons, keywords). Invalid input produces `ERROR` nodes in the CST rather than aborting. Python model files are discovered via subprocess/PyO3 and their SQL is extracted from `@model` decorators before parsing.

**Analyze** -- Salsa provides automatic incremental recomputation. When a file changes, only affected queries re-evaluate. The CST itself is never mutated; semantic information is computed as derived queries (`parse_file`, `model_refs`, `resolve_ref`, `file_diagnostics`, `model_schema`).

**Plan** -- The planner reads CST structure to detect patterns, then emits `Transformation` instructions and new SQL strings. It does not mutate existing CSTs. Planning is optional; models run correctly without it.

**Generate** -- The dialect-aware printer walks the CST in a single forward pass and emits target-specific SQL. Each construct that needs translation is a match arm in the recursive walk. For the native dialect (DuckDB), the printer emits SQL identical to the input modulo `smelt.ref()` resolution.

**Execute** -- Each backend implementation handles DDL (CREATE TABLE AS, views, incremental inserts) and returns `ExecutionResult` with duration, row count, and optional data preview as Arrow `RecordBatch`.

---

## Crate Structure

### Core Pipeline

| Crate | Purpose | Key Types | Sync/Async |
|-------|---------|-----------|------------|
| `smelt-types` | SQL data types shared across crates | `DataType`, `TypedColumn` | sync |
| `smelt-parser` | Rowan-based error-recovery parser | `SyntaxNode`, `SyntaxKind`, `Parse` | sync |
| `smelt-core` | Project config and model discovery | `Config`, `ModelFile`, `ModelDiscovery` | sync |
| `smelt-db` | Salsa incremental queries over parsed models | `Database`, `parse_file`, `model_refs`, `model_schema` | sync |
| `smelt-dialect` | Dialect-aware CST printer and backend capabilities | `SqlDialect`, `BackendCapabilities` | sync |
| `smelt-planner` | Model-graph optimization rules | `Transformation`, `ExecutionStep`, `Opportunity` | sync |

### Execution

| Crate | Purpose | Key Types | Sync/Async |
|-------|---------|-----------|------------|
| `smelt-cli` | Command-line interface (`run`, `explain`, `backbuild`) | -- | async |
| `smelt-backend` | Execution trait and result types | `Backend`, `ExecutionResult` | async |
| `smelt-backend-duckdb` | DuckDB execution | `DuckDbBackend` | async |
| `smelt-backend-spark` | Spark/Databricks execution | `SparkBackend` | async |

### Tools

| Crate | Purpose | Key Types | Sync/Async |
|-------|---------|-----------|------------|
| `smelt-lsp` | Language Server Protocol server | -- | async (tower-lsp) |
| `smelt-state` | Run manifests and interval tracking | `RunManifest`, `IntervalStore` | sync |
| `smelt-ui` | Web dashboard and run execution | -- | async |

### Testing

| Crate | Purpose | Sync/Async |
|-------|---------|------------|
| `smelt-bench` | Benchmarks | standalone |
| `smelt-datagen` | Test data generation | standalone |
| `smelt-parser-compat` | Compatibility tests against pg_query, sqlparser-rs, sqlglot | sync |

### Dependency Graph

```
                      smelt-types
                          |
                      smelt-parser
                          |
                   +------+------+
                   |      |      |
              smelt-core  |  smelt-dialect
                   |      |      |
                   |  smelt-db   |
                   |      |      |
            +------+------+------+
            |      |      |
         smelt-lsp |  smelt-planner
            |      |      |
            |  smelt-cli--+
            |      |
            |  smelt-backend
            |      |
            |   +--+------+
            |   |         |
            |  duckdb   spark
            |  backend  backend
            |
         (LSP binary)
```

!!! note "Why `smelt-dialect` is separate"
    The LSP needs dialect information (e.g., "QUALIFY will be rewritten for PostgreSQL") but must not link against heavy async/native dependencies like Arrow, Tokio, and DuckDB. `smelt-dialect` is a lightweight sync crate that both `smelt-lsp` and `smelt-cli` depend on, without either needing to depend on `smelt-backend`.

---

## Key Design Decisions

### Rowan CST as Single Representation

There is no intermediate IR like DataFusion LogicalPlan. The concrete syntax tree from parsing flows through the entire pipeline. This preserves:

- **Comments and whitespace** -- important for readable generated SQL
- **smelt extensions** -- `smelt.ref()`, `smelt.metric()`, named parameters
- **Original formatting** -- the printer's default arm emits tokens verbatim
- **Error nodes** -- the LSP works with incomplete/invalid code

Avoiding an IR also eliminates two fidelity boundaries (CST-to-IR and IR-to-SQL) that would lose information or require complex round-tripping.

### Salsa for Incremental Computation

[Salsa](https://salsa-rs.github.io/salsa/) tracks query dependencies automatically. When a file changes, only affected queries are recomputed. This is critical for LSP responsiveness -- parsing 1000 models once takes around 1 second, but re-analyzing a single changed file takes around 50ms.

Key Salsa queries in `smelt-db`:

- `parse_file()` -- CST from source text
- `model_refs()` -- ref names and source positions
- `resolve_ref()` -- target model for a ref
- `file_diagnostics()` -- errors and warnings
- `model_schema()` -- inferred column types

### Error-Recovery Parsing

Developers write invalid code most of the time while editing. The parser always produces a tree, even from invalid input. Rowan's `ERROR` nodes allow the LSP to provide diagnostics, completions, and go-to-definition on incomplete code. The parser uses sync points (semicolons, SQL keywords) to recover and continue parsing after errors.

### Two-Graph Architecture

smelt maintains two distinct graph representations:

**Logical graph** -- The user's models as written. Each model maps to a `.sql` file. References between models form a DAG. This graph is never modified by the planner.

**Physical graph** -- The execution plan. The planner may add synthetic nodes (shared materializations, cube split intermediates), remove nodes (fused models), redirect references, and change materialization strategies. The physical graph is what actually gets executed.

This separation means users always see their original model structure, while the execution engine works with an optimized plan.

### Sync Core, Async Edges

All core logic (parsing, analysis, optimization, printing) is synchronous. Async is only at the execution boundary where network I/O happens (backend crates, LSP server via tower-lsp). This keeps the codebase simple, testable, and compatible with Salsa (which is sync).

### Expression Optimization is the Engine's Job

smelt does not attempt predicate pushdown, join reordering, or cost-based optimization within a single query. DuckDB, Spark, and BigQuery all have mature query optimizers for that. smelt's value is in cross-model planning that no single engine can do because it doesn't see the full pipeline.

---

## Planner Rule System

The planner is smelt's key differentiator. It inspects model SQL (via the CST) and the model dependency graph, then produces transformations.

### Transformation Types

Rules emit `Transformation` instructions that describe graph edits:

| Transformation | Purpose |
|----------------|---------|
| `CreateNode` | Add a synthetic intermediate model (shared materialization, cube split temp) |
| `RemoveNode` | Remove a model from execution (fused into another) |
| `RedirectRef` | Point all references from one model to another |
| `SetMaterialization` | Override a model's materialization strategy |
| `SetIncremental` | Mark a model for incremental execution with time partitioning |
| `ReplaceWithPlan` | Replace single-query execution with a multi-step plan |

Multi-step plans use `ExecutionStep` variants: `CreateTemp`, `AppendToTemp`, `FinalQuery`, `DropTemp`.

### Rule Structure

Each rule implements a detect/rewrite pattern:

```rust
// Detection: inspect model SQL and graph structure
pub fn detect(model: &ModelInfo) -> Result<Option<Opportunity>, String>;

// Rewriting: produce transformations for a detected opportunity
pub fn rewrite(model: &ModelInfo) -> Option<Vec<ExecutionStep>>;
```

Current rules:

- **Cube split** -- Detects models with multiple `COUNT(DISTINCT)` aggregations and splits into parallel sub-queries joined on GROUP BY keys, reducing memory pressure.
- **Incremental materialization** -- Detects time-partitioned GROUP BY and generates DELETE+INSERT execution plans.

### Phase Ordering

The planner applies rules in two phases:

1. **Cross-model rules first** -- Graph-level transforms (shared materializations, model fusion, ref redirection) that restructure the dependency graph.
2. **Single-model rules second** -- Execution transforms (cube split, incremental detection) that change how individual models are executed.

### Python Rule Extensibility

Rules can be written in Python via PyO3. Python rules implement the same detect/rewrite pattern and return serialized `Transformation` values. This allows data engineers to write domain-specific optimization rules without modifying Rust code.

---

## Dialect-Aware Printer

The printer in `smelt-dialect` walks the CST in a single forward pass. Each construct that needs dialect translation is a match arm in the recursive walk:

- `smelt.ref()` calls are resolved to `schema.model_name`
- `QUALIFY` is rewritten to a subquery wrapper for backends that lack native support
- Array literal syntax is adapted per dialect
- DATE literals, JSON functions, and other constructs are remapped as needed

The default arm emits tokens verbatim, preserving whitespace and comments. Adding a new rewrite means adding a new match arm. Nested rewrites compose naturally through recursion.

**Identity property**: For the native dialect (DuckDB), the printer emits SQL identical to the input modulo smelt extension resolution. This is a testable correctness invariant.

---

## LSP Integration

The LSP (`smelt-lsp`) is a thin async shell (tower-lsp) over sync Salsa queries. It provides:

- **Parse error diagnostics** with accurate positions
- **Undefined ref diagnostics** for `smelt.ref()` and `smelt.source()`
- **Undeclared column diagnostics** for references to columns not in upstream schemas or `sources.yml`
- **Go-to-definition** for `smelt.ref()`, `smelt.source()`, CTE names, table aliases, and column references (traces through `SELECT *` wildcards)
- **Hover** with type information
- **Column completions** including table alias completions
- **Model name completions** in `smelt.ref()`

The LSP depends on `smelt-dialect` for dialect-specific informational hints (e.g., "QUALIFY will be rewritten to a subquery for PostgreSQL") without linking to any backend binary.
