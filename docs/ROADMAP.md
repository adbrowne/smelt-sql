# smelt Development Roadmap

This document summarizes where each area of smelt stands and what's next. For detailed implementation plans, see [`docs/plans/`](plans/).

## Language & Parser

**Current state**: Full SQL parser with error recovery (Rowan CST), covering SELECT, FROM, JOIN (all types), WHERE, GROUP BY, HAVING, ORDER BY, LIMIT, CTEs, window functions, set operations, subqueries, QUALIFY, PIVOT/UNPIVOT, lambda expressions, array/struct/JSON literals, and all standard operators.

- smelt extensions: `smelt.ref()`, `smelt.metric()`, `smelt.source()` with `=>` named parameters
- Trailing commas in SELECT/GROUP BY
- YAML frontmatter for model configuration
- Python model support via `@model` decorator (subprocess + optional PyO3)
- Multi-dialect superset: PostgreSQL base with DuckDB and Spark features

**Next steps**:
- Metrics DSL (Layer 1 — declarative metric definitions, `smelt.metric()` resolution)
- `smelt.param()` for parameterized models

## Type System

**Current state**: Full type inference for expressions, functions, aggregates, window functions, and cross-model schemas. NULL tracking, row polymorphism (`SELECT *` propagation), and `resolved_model_schema()` Salsa query.

- Property-based testing against DuckDB and Spark (via `smelt-parser-compat`)
- Known divergence registry for backend-specific type differences
- JSON operator type inference

**Next steps**:
- LSP quick-fixes for type errors (CAST suggestions, COALESCE for NULLs)
- Stricter boundary type checking (explicit input/output schemas)

## Planner

**Current state**: `smelt-planner` crate with model-graph-level planning:

- Cube split: splits multi-`COUNT(DISTINCT)` queries into parallel sub-queries
- Incremental materialization: detects time-partitioned GROUP BY, generates DELETE+INSERT
- Temporal dependency inference: analyzes window functions, LAG/LEAD, JOIN intervals to determine lookback/lookahead requirements
- Batch safety analysis: classifies models as FullyBatchSafe/BoundedSafe/PerPartitionOnly
- DAG-aware range computation for backfill planning

**Next steps**:
- Shared materialization detection (multiple models computing same intermediate)
- Model fusion (trivial passthrough models)
- Cost-based optimization (requires backend statistics)

## Backends

**Current state**:
- **DuckDB**: Full implementation — table/view materialization, incremental DELETE+INSERT, bundled (no system install needed)
- **Spark**: Backend trait implementation, type oracle for property tests, dialect-aware SQL generation
- **Dialect printer**: `smelt-dialect` crate — single-pass CST walk emitting target SQL, handles QUALIFY, array literals, DATE literals, JSON function remapping

**Next steps**:
- Spark incremental (MERGE support)
- PostgreSQL backend
- Multi-backend execution in a single run (route models to different engines)

## LSP & Editor Support

**Current state**: Full LSP server (`smelt-lsp`) with Salsa incremental compilation:

- Diagnostics: parse errors, undefined refs, type errors (with accurate positions)
- Go-to-definition for `smelt.ref()` and `smelt.source()`
- Hover with type information and model schemas
- Completions: model names, column names, table alias columns
- Python model awareness (valid ref targets, execution error diagnostics)
- VSCode extension with syntax highlighting and auto-activation

**Next steps**:
- Dialect-specific informational hints ("QUALIFY will be rewritten for PostgreSQL")
- Optimizer opportunity suggestions as code actions
- Rename refactoring across models

## CLI & Execution

**Current state**: `smelt-cli` with full pipeline:

- `smelt run` — execute models with optional `--start`/`--end` for incremental ranges, `--dry-run`, `--full-refresh`, `--auto` (range from interval store)
- `smelt backbuild` — target-focused rebuild with DAG-aware range expansion
- `smelt explain` — dependency graph + JSON export
- `smelt status` — interval coverage and gaps for incremental models
- `smelt history` — run history with model filtering
- `smelt type` — function type signatures
- Smart batching based on batch safety analysis
- `smelt-state` crate for run manifests + interval tracking (`.smelt/` directory)
- Two-stage graph architecture: `LogicalGraph` (user intent) → `PhysicalGraph` (execution plan)
  - `LogicalGraph` with eagerly-resolved config per node (March 26, 2026)
  - `PhysicalGraph` with strategy resolution, ephemeral resolver ownership (March 26, 2026)
  - Graph-level planner transformations: `CreateNode`, `RemoveNode`, `RedirectRef`, `SetMaterialization` (March 26, 2026)
  - `smelt explain` shows physical execution plan with strategies, ephemerals, planner optimizations (March 26, 2026)

**Next steps**:
- `smelt diff` — show pending schema changes
- `smelt validate` — pre-run validation
- Schema evolution with efficient migrations (ALTER+backfill instead of full refresh)

## UI Dashboard

**Current state**: Web dashboard (`smelt-ui`) with React frontend:

- Model graph visualization with dependency explorer
- Run planner: preview execution plan before running
- Run execution from UI with real-time WebSocket progress streaming
- Run history with expandable model details
- Model sidebar with type signatures and metadata

**Next steps**:
- See [docs/plans/20260324-ui-dashboard-expansion.md](plans/20260324-ui-dashboard-expansion.md) for Phases 5-6

## Future / Not Yet Started

- **Metrics DSL**: Declarative metric definitions with semantic metadata (decomposability, temporal behavior)
- **Schema evolution**: Efficient migrations when model definitions change
- **Substrait integration**: Portable plan representation, DataFusion interop
- **Lineage/catalog integration**: Expose model lineage to external catalogs
- **Learning from history**: Use run statistics to suggest optimizations
