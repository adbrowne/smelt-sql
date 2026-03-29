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

### ~~1. Data Testing Framework — `smelt test`~~ ✅ (March 27, 2026)

Fully implemented. See [Data Testing Framework](#data-testing-framework) below for details.

### ~~1. Data Catalog — `smelt docs generate`~~ ✅ (March 29, 2026)

Static data catalog / data dictionary generation. Outputs Markdown (default) or JSON.

- ✅ Per-model pages: description, owner, tags, materialization, columns with inferred types and lineage, upstream/downstream deps, incremental config
- ✅ Column enrichment: merges Salsa type inference with frontmatter descriptions and column-level tests
- ✅ Project index: model table, tag index, execution order
- ✅ JSON format: structured `catalog.json` for machine consumption
- ✅ `--select` filtering reuses existing selector infrastructure
- ✅ Nested subcommand (`smelt docs generate`) for future `smelt docs serve`

See [plan](plans/20260329-docs-generate.md) for details.

### ~~1. Schema Diff — `smelt diff`~~ ✅ (March 29, 2026)

Offline schema change detection. Compares inferred model schemas (from SQL parsing/type inference) against deployed schemas (`.smelt/schemas/`) without requiring a database connection.

- ✅ Per-model diff: column additions, removals, type changes, nullability changes
- ✅ Risk assessment: safe ALTER TABLE vs full refresh vs column removal flag
- ✅ `--select`/`--exclude` filtering reuses existing selector infrastructure
- ✅ `--json` output for CI integration (machine-readable)
- ✅ Exit code 1 when changes detected (CI-friendly)
- ✅ Removed model detection (deployed schema exists but model deleted from code)
- ✅ Per-model target resolution (works with multi-backend projects)

### ~~1. Spark / Databricks Backend~~ 🔄 (March 28, 2026)

Spark backend implemented via PySpark/PyO3 bridge. All Backend trait methods are now functional, connecting to Spark through PySpark's SparkSession.

- ✅ PySpark bridge via PyO3 — thin Python adapter (`spark_adapter.py`) wraps SparkSession
- ✅ SQL execution with zero-copy Arrow result conversion (`pyarrow.Table` → `RecordBatch` via C Data Interface)
- ✅ Table/view materialization (DROP + CREATE TABLE AS, CREATE OR REPLACE VIEW)
- ✅ Incremental support: DELETE+INSERT, MERGE INTO, INSERT OVERWRITE, APPEND
- ✅ Catalog/schema management (three-part names: `catalog.schema.table`)
- ✅ pyo3 upgraded from 0.24 → 0.26 (required for arrow-pyarrow compatibility)
- ✅ Works with local Spark Connect, Databricks Connect, EMR, Dataproc
- 🔮 Integration test parity with DuckDB tests (requires Spark Connect server)
- 🔮 Authentication configuration docs (tokens, OAuth, instance profiles)

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

## Data Testing Framework ✅ (March 27, 2026)

### Test Types
- **CTE isolation tests**: Test a single CTE by mocking all its direct dependencies
- **Whole-model tests**: Test entire model by mocking `smelt.ref()` inputs
- **Singular tests**: Custom SQL assertion tests (`materialization: test`, pass when 0 rows returned)
- **Property-based tests**: Omit columns from inputs → framework generates random values using type inference, runs N times (configurable via `test.cases`)
- **Column-level data quality tests**: `not_null`, `unique`, `accepted_values`, `min`, `max` defined in model frontmatter

### CLI
- `smelt test` with `--select`, `--verbose`, `--show-all`, `--seed` flags
- Tests excluded from `smelt run`/`build`/`explain`
- Example tests across ephemeral_demo, retail_analytics, timeseries projects

### Remaining work
- `smelt docs generate` for data catalog / data dictionary output
- Recursive CTE support in test isolation
- Snapshot/golden file mode (auto-capture expected output)
- LSP validation of test references (`test.model`, `test.target_cte`)
- Seed data integration with tests
- Statically-checkable assertions and type-system-leveraged testing (exploratory)

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
- *See also*: snapshot tests for type inference output ([Code Quality & Hardening](#code-quality--hardening)), type-system-leveraged data testing ([Data Testing Framework](#data-testing-framework))

## Planner

**Current state**: `smelt-planner` crate with model-graph-level planning:

- Cube split: splits multi-`COUNT(DISTINCT)` queries into parallel sub-queries
- Incremental materialization: detects time-partitioned GROUP BY, generates DELETE+INSERT
- Temporal dependency inference: analyzes window functions, LAG/LEAD, JOIN intervals to determine lookback/lookahead requirements
- Batch safety analysis: classifies models as FullyBatchSafe/BoundedSafe/PerPartitionOnly
- DAG-aware range computation for backfill planning

**Deferred**:
- ⏸️ Per-ref upstream filtering — wrapping `smelt.ref()` in filtered subqueries requires column lineage tracing through query AST; currently applies single wider filter range
- ⏸️ Custom time granularities — plugin API for fiscal quarters, 4-4-5 retail calendars; placeholder `Custom` variant exists
- ⏸️ Rule conflict resolution — how planner rules compose when they conflict (e.g., shared sub-expression vs incremental on same model); currently last-transformation-wins

**Next steps**:
- Shared materialization detection (multiple models computing same intermediate)
- Model fusion (trivial passthrough models)
- Cost-based optimization (requires backend statistics)
- Orchestrator integration — Dagster/Airflow plugin API (deferred to separate plan)

## Backends

**Current state**:
- **DuckDB**: Full implementation — table/view materialization, incremental DELETE+INSERT, bundled (no system install needed)
- **Spark**: Full implementation via PySpark/PyO3 bridge (March 28, 2026) — all Backend trait methods implemented, zero-copy Arrow conversion, works with Spark Connect and Databricks Connect. Requires PySpark in Python environment.
- **PostgreSQL**: Not started. Deprioritized in favor of Spark/Databricks.
- **Dialect printer**: `smelt-dialect` crate — single-pass CST walk emitting target SQL, handles QUALIFY, array literals, DATE literals, JSON function remapping

**Deferred**:
- ⏸️ Spark JSON incompatibilities — `TO_JSON(scalar)`, `JSON_CONTAINS`/`@>`/`<@`, `JSON_OBJECT`/`JSON_ARRAY` rewrites; compile-time warnings planned but not yet implemented

**Next steps**:
- ~~Spark/Databricks backend implementation~~ ✅ (March 28, 2026) — see [What's Next #1](#1-spark--databricks-backend)
- ~~Multi-backend execution in a single run~~ ✅ (March 25, 2026) — `BackendRegistry` with per-model `target:` frontmatter override, cross-backend validation
- ~~Cross-engine data exchange~~ ✅ (March 29, 2026) — cross-engine ref resolution via direct Parquet reads (no copy step); DuckDB resolves `smelt.ref('spark_model')` to `read_parquet('{warehouse}/{schema}/{model}/**/*.parquet')`. Example at `examples/multi_engine/`. See [plan](plans/20260328-multi-engine-example.md).
- Integration test parity: run DuckDB integration tests against local Spark Connect
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
- `smelt test` — data testing framework (CTE isolation, whole-model, singular, property-based, column-level tests)
- `smelt type` — function type signatures
- `smelt docs generate` — static data catalog (Markdown/JSON) with column types, lineage, descriptions, tests (March 29, 2026)
- `smelt diff` — offline schema change detection, compares inferred vs deployed schemas without database connection (March 29, 2026)
- Smart batching based on batch safety analysis
- `smelt-state` crate for run manifests + interval tracking (`.smelt/` directory)
- Two-stage graph architecture: `LogicalGraph` (user intent) → `PhysicalGraph` (execution plan)
  - `LogicalGraph` with eagerly-resolved config per node (March 26, 2026)
  - `PhysicalGraph` with strategy resolution, ephemeral resolver ownership (March 26, 2026)
  - Graph-level planner transformations: `CreateNode`, `RemoveNode`, `RedirectRef`, `SetMaterialization` (March 26, 2026)
  - `smelt explain` shows physical execution plan with strategies, ephemerals, planner optimizations (March 26, 2026)

**Next steps**:
- ~~`smelt test`~~ ✅ (March 27, 2026) — see [Data Testing Framework](#data-testing-framework)
- ~~`smelt docs generate`~~ ✅ (March 29, 2026) — see [What's Next](#1-data-catalog--smelt-docs-generate)
- ~~`smelt diff`~~ ✅ (March 29, 2026) — see [What's Next](#1-schema-diff--smelt-diff)
- `smelt validate` — pre-run validation
- Schema evolution with efficient migrations (ALTER+backfill instead of full refresh)

## UI Dashboard ✅ Phases 1-4 (March 24-25, 2026)

**Current state**: Web dashboard (`smelt-ui`) with React frontend and Axum backend:

- Phase 1: Live backend with file watching and WebSocket updates
- Phase 2: Full REST API, batch safety diagnostics, type information in UI
- Phase 3: Run planner with interactive preview, select/exclude with CLI command preview
- Phase 4: Run execution and monitoring with real-time WebSocket progress streaming
- Model graph visualization with dependency explorer
- Run history with expandable model details
- Model sidebar with type signatures and metadata

**Next steps**:
- See [docs/plans/20260324-ui-dashboard-expansion.md](plans/20260324-ui-dashboard-expansion.md) for Phases 5-6

## Ecosystem

**Recent** (March 25-28, 2026):
- ✅ Documentation site for smeltsql.com (MkDocs Material, 15+ pages covering all features)
- ✅ Frontmatter validation with `deny_unknown_fields` (catches typos like `materialized:` vs `materialization:`)
- ✅ Multi-model file discovery with `ModelId` (`--- name: model_name ---` delimiters)
- ✅ Testing documentation: guide, CLI reference, and project structure docs

**Next steps**:
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
