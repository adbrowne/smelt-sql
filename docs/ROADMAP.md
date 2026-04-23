# smelt Development Roadmap

This document summarizes where each area of smelt stands and what's next. For detailed implementation plans, see [`docs/plans/`](plans/).

The **What's Next** section below is the prioritized work queue. Component sections that follow provide context on current state and per-area backlog items.

## What's Next

The items below are the current priority queue. See completed items in [Recently Completed](#recently-completed) below.

### 1. Type Inference, Parser & Ref Resolution Fixes (from smelt_shop validation)

A real-world 19-model ecommerce pipeline ([smelt_shop report](../smelt_report.md)) exposed critical bugs in type inference and ref resolution. These are user-facing correctness issues that block real-world adoption.

**Critical/Major:**
- **Seeds not recognized by `smelt.ref()`** — `resolve_ref()` only searches `all_models()`; seeds aren't in the type-checking model. Workaround: declare seeds as sources in `sources.yml`.
- **Type inference wrong with JOINs on source tables** — multi-table JOIN context produces incorrect CAST wrappers (e.g., VARCHAR→DOUBLE). Workaround: explicit CAST on every output column.
- **CASE expressions produce invalid SQL** — `CAST(? AS TYPE) AS ?` placeholders instead of actual column names/expressions. Workaround: replace CASE with boolean/arithmetic equivalents.
- **`EXTRACT(EPOCH FROM ...)` confuses parser** — FROM inside EXTRACT treated as SQL FROM clause. Workaround: use DuckDB's `EPOCH()` function.
- **CTEs break type inference** — `build_subquery_context()` lacks access to resolved model schemas, can't trace types through CTE chains. Workaround: split CTEs into separate materialized models.
- **Subqueries in FROM don't get ref replacement** — same root cause as CTEs; `smelt.ref()` in subqueries not resolved. Workaround: use top-level JOINs instead.

**Minor:**
- DECIMAL type inference too narrow for division results (overflow > 99)
- FLOAT not handled correctly (DOUBLE works fine)
- Materialization type changes (view↔table) not auto-handled (need manual DROP)

**Root cause pattern:** Issues #5 and #6 share the same root cause — `build_subquery_context()` in `type_inference.rs` is a pure function with no database access, so it can't resolve `smelt.ref()` or `smelt.source()` calls. Fix: thread resolved schemas into context-building functions (consistent with pure-function architecture).

### 2. Packaging — Source Distribution & Python 3.14 Wheels

smelt-sql 0.2.0 has limited wheel availability — only macOS ARM64 (cp314), Windows (cp312), Linux x86_64 (cp311), Linux ARM64 (cp311). No source distribution (sdist). Python 3.14 is the current release and should have wheels on all platforms.

- Publish sdist so users can build from source on any platform
- Add cp314 wheels for all platforms (Linux x86_64, Linux ARM64, Windows, macOS ARM64)
- Ensure CI release workflow covers the full matrix

### 3. Testing Strategy Improvements

The smelt_shop bugs weren't caught because existing tests don't exercise real-world SQL patterns. Four gaps identified:

1. **"Compile and execute" integration test** — For each example workspace, compile every model to target SQL via the dialect printer, then execute against DuckDB. Catches invalid CAST wrappers, broken ref replacement, and code-gen bugs that static analysis (LSP diagnostics) misses.
2. **Complex example workspace** — Add a workspace (or subset of smelt_shop) that exercises JOINs on multiple sources, CASE expressions, CTEs, EXTRACT, subqueries with refs. Becomes both regression test and real-world patterns reference.
3. **Model-level property tests** — Extend proptest suite to generate full model SQL (not just expressions) with JOINs, CTEs, CASE — verify compiled output executes against DuckDB without errors.
4. **Seed integration in type checking** — Seeds are currently a CLI/runtime concept invisible to the type-checking layer. After fixing seed refs, add test coverage for seed schema resolution.

### 4. `smelt check` — LLM-Optimised Diagnostic CLI

Structured diagnostic output designed for LLM consumption. Exposes Smelt's semantic analysis (parse errors, type errors, resolution failures, schema compatibility) via `smelt check --format json` with severity filtering, file/project scope, token budget control (`--budget-lines`), and optional extended context (`--explain`). Replaces the previously planned `smelt validate`. Includes a Claude Code skill and eval harness for empirically tuning diagnostic sufficiency.

See [design doc](plans/20260405-smelt-check.md) for full interface spec, JSON schema, and eval plan.

### 5. Orchestrator Integration

Dagster/Airflow plugin API. `smelt explain --json` already provides the graph structure; next step is a thin adapter layer for orchestrator consumption.

### 6. PostgreSQL Backend

Third backend after DuckDB and Spark. Deprioritized earlier in favor of Spark, now the remaining major backend gap.

### 4. `smelt check` — LLM-Optimised Diagnostic CLI

Structured diagnostic output designed for LLM consumption. Exposes Smelt's semantic analysis (parse errors, type errors, resolution failures, schema compatibility) via `smelt check --format json` with severity filtering, file/project scope, token budget control (`--budget-lines`), and optional extended context (`--explain`). Replaces the previously planned `smelt validate`. Includes a Claude Code skill and eval harness for empirically tuning diagnostic sufficiency.

See [design doc](plans/20260405-smelt-check.md) for full interface spec, JSON schema, and eval plan.

### 5. Orchestrator Integration

Dagster/Airflow plugin API. `smelt explain --json` already provides the graph structure; next step is a thin adapter layer for orchestrator consumption.

### 6. PostgreSQL Backend

Third backend after DuckDB and Spark. Deprioritized earlier in favor of Spark, now the remaining major backend gap.

---

## Recently Completed

### ~~Smelt Functions — Steps 1, 2 & 3~~ ✅ (April 22–24, 2026)

Implemented the first three steps of the smelt-functions experimentation roadmap (Phases 1–18 of [plan](plans/20260422-smelt-functions.md)):

- **Step 1** (Phases 1–6, April 22): `smelt.define` / `smelt.fn.*` parser, Salsa signature index, `Expr<T>` type-reference resolution, Tier 1 body type-check, call-site expansion with single-level frame trace. `safe_divide` end-to-end demo. `examples/functions_demo/` workspace created and registered with CI.
- **Step 2** (Phases 7–12, April 23): `Ordered` constraint, canonical built-in signature registry (~40 functions, generics + variadics), `infer_function_type` rewired through registry, `smelt.extern` declarations, per-declaration frontmatter with `backends:` inference and backend-namespace sugar, multi-level frame rendering in LSP, CAST-enforcement flag on canonical returns.
- **Step 3** (Phases 13–18, April 23–24): `TableExpr` / `AggExpr` / `WindowExpr` / `SelectItems` type-ref grammar; `ExprKind { Scalar, Agg, Window }` with linear subtyping and `SelectItems<K>` kind ceiling; `TableExpr` bare-column row polymorphism with parameters-first scoping and shadow warnings; row-requirement annotations (`TableExpr<{col: Type, ..r}>`); `sessionize` end-to-end with TableExpr output-schema inference; LSP hover for `smelt.define` parameter types (`TableExpr<{...}>` and `Expr<...>` rendered); `add_margin → sessionize` pipeline fixture.

**Deferred during Steps 1–3**: See "Deferred during implementation" appendix in the plan for the full list. Key items: arg-position column resolution (Phase 19+ context binding), structured `Synthesized` marker for default-value provenance, broad TableExpr argument shapes beyond `smelt.ref()`/`smelt.source()`.

### ~~Type Inference, Parser & Ref Resolution Fixes~~ ✅ (April 10, 2026)

All critical/major bugs from the smelt_shop real-world validation report fixed:

- **Seeds as `smelt.ref()` targets** — Seeds are now first-class dep-graph citizens. `resolve_ref()` searches seeds after SQL models; CSV column types inferred and provided to the type-checking layer. No more `sources.yml` workaround.
- **JOIN type inference** — Qualified column refs (`p.col`) no longer fall through to `infer_literal_type()`. Fixed by detecting dot patterns before decimal literal inference.
- **CASE expression column names** — `CAST(? AS TYPE) AS ?` bug fixed; compiler generates `_col1, _col2` deterministic names for unnamed CASE outputs.
- **CASE expression type widening** — `infer_case_expr_type` now promotes across all branches; `promote_types` widens Decimal+Integer to Decimal(38,10).
- **EXTRACT(EPOCH FROM ...)** — New dedicated `EXTRACT_EXPR` syntax kind in the parser handles `EXTRACT(field FROM expr)` without treating the FROM keyword as SQL FROM.
- **CTE type inference** — `parse_when_clause()` fixed to use `parse_or_expr()`, enabling full logical expressions in CASE WHEN.
- **Subquery ref replacement** — Subquery type inference now clones context and processes inner FROM before calling `infer_select_column_types`.
- **FLOAT→DOUBLE normalization** — `CAST(x AS FLOAT)` infers as DOUBLE; `float_division` and `cast_float_as_double` divergences documented.
- **Materialization type changes** — `execute_model()` now drops both table and view before creating either, handling view↔table transitions automatically.
- **Datagen geometric min** — `GeneratorSpec::Geometric` accepts optional `min: i32` to prevent zero values.

See [plan](plans/20260409-smelt-shop-fixes.md) for full details.

### ~~Packaging — Source Distribution & Python 3.14 Wheels~~ ✅ (April 10, 2026)

- Added `build-sdist` job to release workflow using `maturin sdist`
- sdist included in PyPI and TestPyPI publish steps
- `bindings = "bin"` in pyproject.toml produces `py3-none-{platform}` wheels, compatible with Python 3.9–3.14 on all platforms

### ~~Testing Strategy Improvements~~ ✅ (April 10, 2026)

- Added `examples/ecommerce/` workspace (19 models, 2 seeds, 3 sources) as regression scaffold
- Added `ecommerce_no_diagnostics` test to `example_diagnostics.rs`
- Added `ecommerce_execution.rs` compile-and-execute integration test against DuckDB
- Property tests cover CTEs, set operations, joins, and type inference across full model patterns

### ~~LSP Refactorings & Code Actions~~ ✅ (April 5-6, 2026)

Full refactoring support in the LSP: rename (CTEs, models, sources, columns with cross-file lineage tracing), code actions (CAST fixes, create model, add source/column, extract CTE, inline CTE), and find-references. All implemented as pure functions in smelt-db with thin LSP wrappers. Also fixed arrow 57→58 version mismatch and extracted duplicated functions to shared crates.

See [plan](plans/20260405-lsp-refactorings.md) for details.

### ~~LSP Goto-Definition & Column Diagnostics~~ ✅ (April 3-4, 2026)

Major LSP expansion: goto-definition now covers sources, CTEs, columns, and qualified references. Undeclared column reference diagnostics added. Python model LSP integration with real `ProjectContext`. Multiple stability fixes.

See [LSP & Editor Support](#lsp--editor-support) below for full details.

### ~~Code Quality & Hardening~~ ✅ (March 28, 2026)

All four sub-items completed:
- ✅ Snapshot tests: 30 `insta` tests for `smelt-dialect` covering all dialect rewrite paths
- ✅ CLI decomposition: `main.rs` split from 2,656 → 339 lines + 12 per-subcommand modules
- ✅ Structured logging: `tracing` crate replaces ~90 `println!`/`eprintln!` calls across 14 files
- ✅ unwrap() audit: ~35 production `unwrap()` → `expect("reason")` across 13 files

See [Code Quality & Hardening](#code-quality--hardening) below for details.

### ~~Data Testing Framework — `smelt test`~~ ✅ (March 27, 2026)

Fully implemented. See [Data Testing Framework](#data-testing-framework) below for details.

### ~~Data Catalog — `smelt docs generate`~~ ✅ (March 29, 2026)

Static data catalog / data dictionary generation. Outputs Markdown (default) or JSON.

- ✅ Per-model pages: description, owner, tags, materialization, columns with inferred types and lineage, upstream/downstream deps, incremental config
- ✅ Column enrichment: merges Salsa type inference with frontmatter descriptions and column-level tests
- ✅ Project index: model table, tag index, execution order
- ✅ JSON format: structured `catalog.json` for machine consumption
- ✅ `--select` filtering reuses existing selector infrastructure
- ✅ Nested subcommand (`smelt docs generate`) for future `smelt docs serve`

See [plan](plans/20260329-docs-generate.md) for details.

### ~~Schema Diff — `smelt diff`~~ ✅ (March 29, 2026)

Offline schema change detection. Compares inferred model schemas (from SQL parsing/type inference) against deployed schemas (`.smelt/schemas/`) without requiring a database connection.

- ✅ Per-model diff: column additions, removals, type changes, nullability changes
- ✅ Risk assessment: safe ALTER TABLE vs full refresh vs column removal flag
- ✅ `--select`/`--exclude` filtering reuses existing selector infrastructure
- ✅ `--json` output for CI integration (machine-readable)
- ✅ Exit code 1 when changes detected (CI-friendly)
- ✅ Removed model detection (deployed schema exists but model deleted from code)
- ✅ Per-model target resolution (works with multi-backend projects)

### ~~Schema Evolution~~ ✅ (March 30, 2026)

Efficient schema migrations using ALTER TABLE + DEFAULT values instead of full table refresh.

- ✅ Column `default:` in frontmatter — NOT NULL column additions use `ALTER TABLE ADD COLUMN ... DEFAULT val` instead of full refresh
- ✅ Column `backfill:` in frontmatter — SQL expression for UPDATE backfill after ALTER TABLE ADD COLUMN
- ✅ `schema_evolution: { strategy: full_refresh }` — opt out of ALTER-based migration per model
- ✅ Nullable-to-NOT-NULL with default — `UPDATE ... WHERE IS NULL` + `ALTER SET NOT NULL`
- ✅ `smelt diff` shows migration plan with defaults (ALTER with DEFAULT instead of full refresh)

### ~~Schema Evolution — Complex Types~~ ✅ (April 5, 2026)

Production schema evolution for nested/complex types (Struct, Array, Map). Previously, any change to a complex type column triggered a full table refresh.

- ✅ `parse_type()` extended for `STRUCT(...)`, `TYPE[]`, `MAP(K, V)` with recursive nesting
- ✅ `Map(Box<DataType>, Box<DataType>)` variant added to `DataType`
- ✅ Recursive type normalization (`DataType::normalize()`)
- ✅ Structural diff for complex types — field-level additions, removals, type widening, nested changes
- ✅ Safe widening rules for nested types (e.g., `INTEGER` → `BIGINT` inside a struct)
- ✅ Abstract `SchemaOperation` enum for backend-agnostic migration planning
- ✅ DuckDB DDL generation: struct dot-notation, `struct_pack` rewrites, `list_transform` for array-of-struct
- ✅ Spark DDL generation: `mergeSchema` for safe additions, `TableRewrite` for unsupported operations
- ✅ Table format config (`format: delta|parquet`) at target and model level
- ✅ `--allow-full-refresh` CLI gate for expensive operations
- ✅ `default:` changed from YAML value to SQL expression string (breaking change)
- ✅ Identifier quoting for SQL keywords and special characters
- ✅ Graceful fallback for unparseable type strings with warnings
- ✅ Round-trip verification: `DataType` → `to_sql()` → `parse_type()` → `DataType`
- ✅ User-facing documentation on smeltsql.com (schema-evolution guide, backend capability matrix)

See [plan](plans/20260405-schema-evolution-complex-types.md) for details.

### ~~Spark / Databricks Backend~~ ✅ (March 28, 2026)

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

**Current state**: Full SQL parser with error recovery (Rowan CST), covering SELECT, FROM, JOIN (all types), WHERE, GROUP BY, HAVING, ORDER BY, LIMIT, CTEs, window functions, set operations, subqueries, QUALIFY, lambda expressions, array/struct/JSON literals, and all standard operators.

- smelt extensions: `smelt.ref()`, `smelt.metric()`, `smelt.source()` with `=>` named parameters
- Trailing commas in SELECT/GROUP BY
- YAML frontmatter for model configuration
- Python model support via `@model` decorator (subprocess + optional PyO3)
- Multi-dialect superset: PostgreSQL base with DuckDB and Spark features
- PIVOT/UNPIVOT: rejected with diagnostic error (not yet supported, March 31, 2026)
- Parser structural assertion tests and AST accessor bug fixes (April 3, 2026)
- Fixed bare-token problem and implicit alias detection (April 3, 2026)

**Next steps**:
- ~~Smelt Functions Steps 1–3~~ ✅ (April 22–24, 2026) — `smelt.define`, `smelt.fn.*`, `TableExpr`, call-site type checking, LSP hover. Steps 4–8 (context binding, Tier 2/3, PASSING, planner, struct row vars) remain. See [plan](plans/20260422-smelt-functions.md) and [discussion paper](research/20260413-smelt-functions.md).
- Metrics DSL (Layer 1 — declarative metric definitions, `smelt.metric()` resolution)
- `smelt.param()` for parameterized models
- PIVOT/UNPIVOT support (currently rejected with diagnostic)

## Type System

**Current state**: Full type inference for expressions, functions, aggregates, window functions, and cross-model schemas. NULL tracking, row polymorphism (`SELECT *` propagation), and `resolved_model_schema()` Salsa query.

- Property-based testing against DuckDB and Spark (via `smelt-parser-compat`)
- Comprehensive generator coverage (March 29, 2026): 12 expression kinds (IS NULL, comparisons, unary NOT/minus, EXISTS, LIKE/ILIKE, regex, scalar subqueries, mixed-type binary ops, `::` cast), 5 query shapes (Scalar, GroupBy, GroupByHaving, GroupByWindow, Distinct), 10 base types (incl. Time, Interval), window frame specs
- LIKE/ILIKE parser support with type inference
- Known divergence registry for backend-specific type differences
- JSON operator type inference

**Next steps**:
- ~~LSP quick-fixes for type errors (CAST suggestions)~~ ✅ (April 5, 2026) — see [LSP Refactorings](#lsp-refactorings--code-actions--april-5-6-2026)
- LSP quick-fixes for COALESCE suggestions on NULLs
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
- Three-level rule architecture: (1) Logical→Logical transforms with functions as opaque typed nodes, (2) Logical→Physical with strategy-dependent function expansion, (3) Physical→Execution plan with multi-statement orchestration. See [smelt functions discussion paper](research/20260413-smelt-functions.md) §8.
- Function-aware optimizations: join elimination for unused 1:1 LEFT JOINs, predicate pushdown into function blocks, cross-function fusion
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

- Diagnostics: parse errors, undefined refs, type errors, undeclared column references (with accurate positions)
- Go-to-definition for `smelt.ref()`, `smelt.source()`, CTEs, columns, and qualified references (e.g., `t.column`)
- CTE wildcard tracing (`SELECT *` through CTE chains)
- Hover with type information and model schemas
- Completions: model names, column names, table alias columns
- Python model awareness: real `ProjectContext` passed to Python models in LSP, valid ref targets, execution error diagnostics
- `sources.yml` live reload (changes update LSP without restart)
- Salsa 0.26 with `#[salsa::tracked]` free functions and `cycle_initial` fixpoint iteration (upgraded from 0.16)
- Find references for models, sources, and CTEs
- Rename: CTEs (single-file), models (cross-file with file rename), sources (cross-file + YAML), columns (full lineage tracing)
- Code actions: CAST quick-fixes, create model, add source/column to YAML, extract CTE, inline CTE
- VSCode extension with syntax highlighting and auto-activation
- CI verification: example workspaces checked for zero LSP diagnostics

**Recent** (April 3-4, 2026):
- ✅ Expanded goto-definition to sources, CTEs, columns, and qualified references
- ✅ CTE wildcard tracing for `SELECT *` column resolution
- ✅ Diagnostics for undeclared column references
- ✅ Python model LSP integration: real `ProjectContext` enables cross-boundary type inference
- ✅ Fixed LSP crash from Salsa cycle detection during memo validation
- ✅ Upgraded Salsa 0.16 → 0.26: `#[salsa::tracked]` free functions, `#[salsa::input]` structs, `#[salsa::accumulator]` diagnostics, `cycle_initial` fixpoint iteration; removed `catch_unwind` workaround (April 18, 2026)
- ✅ Fixed `sources.yml` changes not updating LSP until reload
- ✅ Fixed 35 LSP diagnostics across example workspaces + CI verification gate
- ✅ Fixed Python model `E2BIG` error on large projects and PyO3 `dict_items` extraction

**Next steps**:
- Dialect-specific informational hints ("QUALIFY will be rewritten for PostgreSQL")
- Optimizer opportunity suggestions as code actions
- Code action: extract to model (promote subquery/CTE to a new smelt model)

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
- `smelt check` — LLM-optimised diagnostic CLI ([design doc](plans/20260405-smelt-check.md))
- ~~Schema evolution with efficient migrations~~ ✅ (March 29, 2026) — see [What's Next](#1-schema-evolution)

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

**Recent** (March 25 – April 4, 2026):
- ✅ Documentation site for smeltsql.com (MkDocs Material, 15+ pages covering all features)
- ✅ Frontmatter validation with `deny_unknown_fields` (catches typos like `materialized:` vs `materialization:`)
- ✅ Multi-model file discovery with `ModelId` (`--- name: model_name ---` delimiters)
- ✅ Testing documentation: guide, CLI reference, and project structure docs
- ✅ ACE-FCA workflow: slash commands, tutorial, and artifact directories for structured development (March 31, 2026)
- ✅ SQL dialect analysis report: confirmed multi-dialect superset approach is sound (March 30-31, 2026)
- ✅ System DuckDB as default build mode — faster builds, no bundled C++ compilation (April 3, 2026)
- ✅ CI verification: example workspaces checked for zero LSP diagnostics (April 3, 2026)
- ✅ CI release builds fixed for bundled-duckdb feature (April 4, 2026)

- ✅ smelt-datagen bundled in `smelt-sql` PyPI wheel and standalone archives (April 9, 2026)
- ✅ smelt-datagen documentation: guide page on smeltsql.com covering all features (April 9, 2026)
- ✅ New datagen generators: `date`, `timestamp`, and `string_pattern` for realistic test data (April 9, 2026)

**Next steps**:
- Pre-built binaries via GitHub Releases (dev-release.yml workflow exists)
- Source distribution (sdist) + Python 3.14 wheels for all platforms (see [What's Next #2](#2-packaging--source-distribution--python-314-wheels))
- Datagen: geometric distribution `min` parameter (currently can produce 0, unsuitable for quantity fields)
- dbt-to-smelt cheat sheet showing common pattern equivalents
- Publish Python SDK to PyPI (currently TestPyPI only)
- Generic LSP configuration guides for Neovim, Emacs, and JetBrains

## Future / Exploration

Items here are interesting design problems without committed timelines.

- **External models in the graph**: Non-smelt models (e.g., PySpark jobs, legacy pipelines) as first-class DAG participants. User-annotated output schema and temporal behavior (partition column, granularity). Configurable execution: smelt-triggered (command/webhook) or externally-managed. Enables gradual migration and mixed-technology pipelines. Smelt's backbuild range computation would account for these models' declared temporal mappings. Declaration format needs design work.
- **Virtual environments / plan-apply workflow**: Compare schemas across dev/prod without materializing; require approval before execution. Interesting state management problem — smelt's logical/physical graph split could enable lightweight virtual environments.
- **OpenLineage / column-level lineage**: Export model and column-level lineage in OpenLineage format for catalog integration (DataHub, Amundsen, Atlan). Internal lineage tracking partially exists — interesting graph analysis problem.
- **Substrait integration**: Portable plan representation, DataFusion interop
- **Smelt Functions — Steps 4–8** (context binding, Tier 2/3 checking, PASSING clauses, planner-rule API, struct row vars): Steps 1–3 are ✅ complete (April 2026). Remaining steps continue the experimentation roadmap. See [plan](plans/20260422-smelt-functions.md) and [discussion paper](research/20260413-smelt-functions.md).
- **Learning from history**: Use run statistics to suggest optimizations
