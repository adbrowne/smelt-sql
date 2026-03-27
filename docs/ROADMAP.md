# smelt Development Roadmap

This document summarizes where each area of smelt stands and what's next. For detailed implementation plans, see [`docs/plans/`](plans/).

The **What's Next** section below is the prioritized work queue. Component sections that follow provide context on current state and per-area backlog items.

## What's Next

### ~~1. Code Quality & Hardening~~ ✅ (March 28, 2026)

All four sub-items completed:
- ✅ Snapshot tests: 30 `insta` tests for `smelt-dialect` covering all dialect rewrite paths
- ✅ CLI decomposition: `main.rs` split from 2,656 → 339 lines + 12 per-subcommand modules
- ✅ Structured logging: `tracing` crate replaces ~90 `println!`/`eprintln!` calls across 14 files
- ✅ unwrap() audit: ~35 production `unwrap()` → `expect("reason")` across 13 files

See [Code Quality & Hardening](#code-quality--hardening) below for details.

### 1. Data Testing Framework — `smelt test`

Every production-oriented perspective in the codebase review identified the absence of a data testing framework as the single biggest gap. Beyond filling that gap, smelt's architecture enables testing approaches that aren't possible in dbt or SQLMesh.

- Schema-level assertions: not_null, unique, relationships, custom SQL predicates
- `smelt docs generate` for data catalog / data dictionary output
- Ideas to explore (details in a future plan doc):
  - Property-based data tests — leverage proptest infrastructure for user data validation
  - Statically-checkable assertions — verify invariants at compile time (e.g., input/output row count matching for filter-free models)
  - Type-system-leveraged testing — auto-generate constraint checks from inferred schemas

### 2. Spark / Databricks Backend

The current Spark backend is a 267-line stub where every method returns an error. Making this real proves out the multi-backend architecture — a key differentiator — and enables running actual workloads on Databricks.

- Implement Spark Connect integration (or Databricks SQL REST API)
- Table materialization with Delta Lake format support
- Incremental support: INSERT OVERWRITE for partitioned tables, MERGE INTO
- Cluster and authentication configuration (local Spark, Databricks tokens)
- Test parity: run existing DuckDB integration tests against local Spark

---

## Code Quality & Hardening ✅ (March 28, 2026)

### Structured Logging ✅

- `tracing` crate with `EnvFilter` (controlled via `RUST_LOG` env var)
- ~90 `println!`/`eprintln!` calls converted to `tracing::info!`/`debug!`/`warn!` across 14 files
- Program output (tables, JSON, test results) kept as `println!` for piping

### Error Handling ✅

- ~35 production `unwrap()` calls replaced with `expect("reason")` across 13 files
- Focused on smelt-cli, smelt-db, smelt-core, smelt-backend-duckdb
- Test code left as-is (idiomatic Rust)
- Remaining `unwrap()` calls are in test code or already have proper error handling

### CLI Decomposition ✅

- `main.rs` split from 2,656 → 339 lines (arg structs + dispatch only)
- 11 per-subcommand modules under `src/commands/` (run, backbuild, seed, build, status, history, explain, table, type, ui, test)
- Shared utilities extracted to `src/helpers.rs` (352 lines)

### Snapshot Testing ✅

- 30 `insta` snapshot tests for `smelt-dialect` printer
- Covers all dialect rewrite paths: QUALIFY, ARRAY, DATE, `::` cast, trailing comma, function remapping, ref/source resolution, ephemeral refs, combined rewrites
- All three dialects tested: DuckDB, SparkSQL, PostgreSQL

---

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
- *See also*: snapshot tests for type inference output ([Code Quality & Hardening](#code-quality--hardening-)), type-system-leveraged data testing ([What's Next #1](#1-data-testing-framework--smelt-test))

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
- **Spark**: Architectural scaffolding only — backend trait, type oracle for property tests, dialect-aware SQL generation in `smelt-dialect`. **The execution backend is not yet implemented** (every method returns an error). See [What's Next #2](#2-spark--databricks-backend) for implementation plan.
- **PostgreSQL**: Not started. Deprioritized in favor of Spark/Databricks.
- **Dialect printer**: `smelt-dialect` crate — single-pass CST walk emitting target SQL, handles QUALIFY, array literals, DATE literals, JSON function remapping

**Next steps**:
- Spark/Databricks backend implementation ([What's Next #2](#2-spark--databricks-backend))
- Multi-backend execution in a single run (route models to different engines)
- *Deferred*: PostgreSQL backend

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
- `smelt test` — data testing framework ([What's Next #1](#1-data-testing-framework--smelt-test))
- `smelt docs generate` — data catalog output ([What's Next #1](#1-data-testing-framework--smelt-test))
- ~~CLI decomposition into per-subcommand modules~~ ✅ (March 28, 2026)
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

## Ecosystem

- Pre-built binaries via GitHub Releases (dev-release.yml workflow exists)
- dbt-to-smelt cheat sheet showing common pattern equivalents
- Publish Python SDK to PyPI (currently TestPyPI only)
- Generic LSP configuration guides for Neovim, Emacs, and JetBrains

## Future / Exploration

Items here are interesting design problems without committed timelines.

- **External models in the graph**: Non-smelt models (e.g., PySpark jobs, legacy pipelines) as first-class DAG participants. User-annotated output schema and temporal behavior (partition column, granularity). Configurable execution: smelt-triggered (command/webhook) or externally-managed. Enables gradual migration and mixed-technology pipelines. Smelt's backbuild range computation would account for these models' declared temporal mappings. Declaration format needs design work.
- **Virtual environments / plan-apply workflow**: Compare schemas across dev/prod without materializing; require approval before execution. Interesting state management problem — smelt's logical/physical graph split could enable lightweight virtual environments.
- **OpenLineage / column-level lineage**: Export model and column-level lineage in OpenLineage format for catalog integration (DataHub, Amundsen, Atlan). Internal lineage tracking partially exists — interesting graph analysis problem.
- **Metrics DSL**: Declarative metric definitions with semantic metadata (decomposability, temporal behavior)
- **Schema evolution**: Efficient migrations when model definitions change (ALTER TABLE + selective backfill)
- **Substrait integration**: Portable plan representation, DataFusion interop
- **Reusable SQL patterns**: dbt solves SQL reuse with Jinja macros, which smelt deliberately avoids. The problem is real — common patterns like date spine generation, surrogate key hashing, and standard metric calculations get copy-pasted across models. No clear solution yet. Possible directions: parameterized SQL includes, a lightweight macro system that doesn't compromise the parser, leveraging Python models as generators, or something entirely different. Open design problem.
- **Learning from history**: Use run statistics to suggest optimizations
