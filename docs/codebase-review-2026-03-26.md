# smelt Codebase Review Report

**Date**: March 26, 2026
**Version reviewed**: 0.1.0
**Codebase**: ~55,000 LOC Rust, 16 crates, 919 tests
**Review methodology**: Multi-perspective analysis from 10 professional viewpoints
**Author**: Independent review

---

## Executive Summary

smelt is a technically ambitious and well-architected data transformation framework that brings compiler-grade tooling to the data engineering space. Its core innovations -- a lossless Rowan CST parser with error recovery, Salsa-powered incremental computation for real-time LSP support, and automatic incrementalization through semantic analysis -- represent genuine advances over both dbt and SQLMesh.

**Top 3 strengths:**
1. **Parser and LSP architecture** follows the rust-analyzer pattern (Rowan + Salsa), delivering real-time type checking, hover information, and go-to-definition that no other data transformation tool offers
2. **Type system with NULL tracking** catches entire categories of SQL bugs at authorship time -- implicit coercion, ambiguous columns, nullable join columns -- that dbt and SQLMesh only surface at runtime
3. **Logical/physical separation** is a sound architectural bet: users write pure SQL business logic, and the framework handles incrementalization, optimization, and backend-specific translation

**Top 3 risks:**
1. **Bus factor of 1**: single author, no community contributors, no governance structure
2. **DuckDB is the only functional backend**: Spark is a 267-line stub with every method returning an error; PostgreSQL does not exist
3. **No data testing framework**: the single most requested feature across every production-oriented perspective

**Verdict spectrum**: Ranges from "adopt now" for DuckDB-local analytics work, to "hard pass" for Spark-dependent teams. Most perspectives recommend "wait 6-12 months" for production adoption.

---

## Project Overview

smelt is a data transformation framework that compiles SQL models with semantic extensions (`smelt.ref()`, `smelt.source()`) into optimized, backend-specific execution plans. Unlike dbt's Jinja template approach, smelt parses and understands SQL semantics, enabling static analysis, cross-model type inference, and automatic optimization.

**Compilation pipeline:**
```
SQL models (with YAML frontmatter)
  -> Lexer/Parser (Rowan CST with error recovery)
  -> Salsa incremental DB (caching, invalidation)
  -> Type inference (cross-model schema propagation)
  -> Planner (cube split, incremental detection, batch safety)
  -> Logical Graph -> Physical Graph
  -> Dialect-specific SQL generation
  -> Backend execution (DuckDB / Spark stub)
```

**Key design decisions:**
- SQL-first: no Jinja templates, no macro system -- models are pure SQL with `smelt.*` function extensions
- YAML frontmatter for per-model configuration (materialization, incremental settings, tags, owner)
- `smelt.yml` for project-level configuration with target environments
- Named parameters via `=>` operator (SQL:2003 standard)
- Python models supported via `@model` decorator for dynamic SQL generation

---

## Perspective 1: Current dbt User Considering a Switch

> "I've been running dbt for three years. My project has 200+ models, custom macros, and a test suite. What does smelt offer me?"

**What impresses:**
The elimination of Jinja is immediately compelling. Where dbt requires `{% if is_incremental() %}` blocks that mix execution logic into business logic, smelt models are pure SQL. The incremental configuration lives in YAML frontmatter, and the framework handles the WHERE clause injection and partition management automatically. The batch safety analysis in `crates/smelt-planner/src/rules/incremental.rs` -- which classifies models as FullyBatchSafe, BoundedSafe, or PerPartitionOnly based on semantic analysis of window functions and JOINs -- is genuinely novel.

The LSP is a step change in developer experience. Hovering over a column shows its inferred type with nullability. Go-to-definition on `smelt.ref('upstream')` jumps to the model file. Parse errors appear in real-time with error recovery, so incomplete SQL still gets partial diagnostics. dbt has nothing comparable.

**What concerns:**
The absence of a data testing framework is a dealbreaker for production dbt projects. There is no `smelt test`, no schema-level assertions (not_null, unique, relationships), no custom SQL tests. The 919 tests in the codebase test the *framework*, not user *data*. There is also no documentation generation (`dbt docs generate`), no package system (dbt-utils, dbt-expectations), no snapshots/SCD Type 2, no hooks, and no macros for reusable SQL patterns.

**Verdict: Wait**

Monitor until data testing and documentation generation land. The LSP and type safety are compelling reasons to switch eventually, but the missing testing framework means no production safety net.

**Recommendations:**
1. Prioritize `smelt test` with schema assertions as the single highest-impact feature
2. Create a dbt-to-smelt migration cheat sheet showing pattern equivalents
3. Add `smelt docs generate` for data catalog output

---

## Perspective 2: Director of Engineering Considering Adoption

> "One of my teams wants to adopt this. I need to understand the risk profile."

**What impresses:**
The engineering quality is high. 55K LOC of Rust with clear crate separation, 919 tests including property-based testing against DuckDB oracles, 2 fuzz targets for parser robustness, and 9 CI workflows covering lint, test, fuzz, compatibility, and benchmarking. The MIT license avoids vendor lock-in. The architecture documentation in `docs/` is thorough enough that a new engineer could understand the system design.

**What concerns:**
This is a single-author project with no community governance. There are no external contributors, no CONTRIBUTING.md, no community channels (Slack/Discord), and no known production deployments. If the author becomes unavailable, the project stalls.

The Spark backend is listed as a feature but is a 267-line stub where every method returns an error. Presenting this to stakeholders as "multi-backend support" would be misleading. There is no orchestrator integration (Airflow, Dagster, Prefect), no structured logging (320 `println!` calls vs 8 `tracing::` calls), and no security/credential management.

**Verdict: Pass (for now)**

Too early for organizational adoption. The technical foundation is strong, but the operational maturity is not there.

**Recommendations:**
1. Consider a limited pilot only if the team exclusively uses DuckDB for local analytics
2. Watch the GitHub repository for community growth signals before re-evaluating
3. If seriously interested, engage the author directly about roadmap and support

---

## Perspective 3: Current SQLMesh User Considering a Switch

> "I use SQLMesh for its virtual environments and plan/apply workflow. What does smelt do better?"

**What impresses:**
The parser quality exceeds SQLMesh's sqlglot-based approach. smelt's Rowan CST preserves every byte of input (whitespace, comments) and recovers gracefully from errors, while sqlglot produces a lossy AST that can drop formatting. The temporal dependency analysis in `crates/smelt-planner/src/analysis/temporal.rs` -- which automatically infers lookback requirements from window functions and JOIN intervals -- is innovative and has no SQLMesh equivalent.

**What concerns:**
SQLMesh's two killer features are absent. Virtual environments (comparing schemas across dev/prod without materializing) do not exist. The plan/apply workflow (showing pending changes and requiring approval before execution) is not implemented. Column-level lineage tracking, which SQLMesh provides automatically, exists only as an incomplete internal data structure. The backend situation is also weaker: SQLMesh supports BigQuery, Snowflake, Databricks, Postgres, and more; smelt has DuckDB.

**Verdict: Pass**

SQLMesh is more mature in the areas that matter for production workflows. smelt's parser and type system are technically stronger, but those advantages do not compensate for the missing workflow features.

**Recommendations:**
1. Watch smelt's LSP development -- if editor experience is your primary pain point, smelt's LSP is already ahead
2. The temporal dependency analysis concept is worth following as it matures

---

## Perspective 4: Senior Data Architect

> "I'm evaluating the system design for scalability, integration patterns, and long-term viability."

**What impresses:**
The logical/physical graph separation in `crates/smelt-cli/src/logical_graph.rs` and `physical_graph.rs` is textbook good architecture. User intent (LogicalGraph with model definitions and materializations) is cleanly separated from execution strategy (PhysicalGraph with partition boundaries, batch composition, and backend routing). The planner can apply graph-level transformations (CreateNode, RemoveNode, RedirectRef, SetMaterialization) that modify execution without touching user models.

The backend abstraction in `crates/smelt-backend/src/lib.rs` is well-designed with a 14-method trait, capability negotiation via `BackendCapabilities`, and dialect-specific SQL generation in a separate `smelt-dialect` crate. The type system's cross-model schema propagation -- where column types flow through refs and JOINs with NULL promotion on LEFT JOIN -- is architecturally sound.

**What concerns:**
No data lineage export exists. Column-level lineage tracking is partially implemented internally but there is no API to export to catalog tools (DataHub, Amundsen, Atlan) or OpenLineage format. Schema evolution has no ALTER TABLE migration path -- schema changes require full refresh. The DuckDB incremental strategy (DELETE + INSERT) is not atomic; a failure between the DELETE and INSERT could cause data loss. Multi-backend routing within a single run is architecturally supported but not implemented.

**Verdict: Wait with strong interest**

The architecture is sound and scalable. The logical/physical split and backend abstraction are well-designed foundations that would be expensive to retrofit. Worth monitoring closely.

**Recommendations:**
1. Implement OpenLineage export for catalog integration
2. Add transactional safety to DuckDB incremental operations (wrap in a transaction)
3. Prioritize Substrait integration for multi-engine portability

---

## Perspective 5: Senior Analytics Engineer

> "I write 5-10 models a week and care about productivity. Will smelt make me faster?"

**What impresses:**
The LSP experience is genuinely delightful. Column completions after typing a table alias (`o.` shows order columns), hover showing `order_date: DATE NULL (from LEFT JOIN, promoted to nullable)`, and go-to-definition jumping to upstream models -- this is IDE-quality tooling that analytics engineers have never had for SQL transformation projects. The error recovery parser means you get diagnostics while typing, not just after saving.

The YAML frontmatter keeps configuration close to the SQL it applies to. The `smelt status` command showing interval coverage for incremental models, and `smelt explain` previewing the execution plan, provide visibility that usually requires custom tooling.

**What concerns:**
There is no way to create reusable SQL snippets. dbt macros are messy but they solve a real need -- common date spine generation, surrogate key hashing, standard metric calculations. smelt has Python models for dynamic SQL generation, but that requires switching languages. The learning curve for `smelt.ref('model')` vs `{{ ref('model') }}` is minor, but the lack of community examples and tutorials makes onboarding harder. The VSCode extension exists but there is no published guide for Neovim, Emacs, or JetBrains configuration.

**Verdict: Adopt for DuckDB local projects**

For DuckDB-based analytics work, the LSP and type checking provide genuine daily productivity gains that no other tool offers. Not ready for production pipelines due to missing testing.

**Recommendations:**
1. Publish a generic LSP configuration guide for editors beyond VSCode
2. Add a `description` field rendering that surfaces model documentation in the LSP hover
3. Consider a lightweight SQL reuse mechanism (perhaps parameterized SQL includes)

---

## Perspective 6: Senior Data Engineer (PySpark / Scala Spark)

> "My team runs 2,000+ Spark jobs daily on Databricks. I need reliable Spark integration."

**What impresses:**
The backend trait design is clean and extensible. `SqlDialect::SparkSQL` exists with correct capability flags (supports INSERT OVERWRITE, lacks QUALIFY, different array literal syntax). The dialect printer in `crates/smelt-dialect/` handles QUALIFY rewriting to WHERE-subquery form and remaps function names across dialects. The `smelt-parser-compat` crate runs cross-dialect type conformance tests. The *vision* of multi-backend execution is architecturally supported.

**What concerns:**
The Spark backend at `crates/smelt-backend-spark/src/lib.rs` is 267 lines where every method -- `connect()`, `execute_sql()`, `create_table()`, `drop_table()`, all of them -- returns `Err(BackendError::Other { message: "Spark backend not yet implemented" })`. This is not early-stage. This is unstarted. There is no Spark Connect dependency, no Delta Lake/Iceberg support, no cluster configuration, no partitioning strategy (Spark's PARTITIONED BY), and no resource management.

The Python model support executes via subprocess, not PySpark. The PyO3 embedding is for the planner SDK, not for Spark integration.

**Verdict: Hard Pass**

Spark support does not exist. The stub communicates intent but delivers nothing. There is no foundation to build on here.

**Recommendations:**
1. Be transparent about Spark status in documentation -- listing it alongside DuckDB as a "backend" is misleading
2. When implementing: start with Databricks SQL REST API (lower complexity than Spark Connect) and Delta Lake table format
3. Consider Spark Connect via the spark-connect-rs crate when it matures

---

## Perspective 7: Data Analyst Maintaining a Small Project

> "I have 15 models that transform CSVs into dashboards using DuckDB. I want something simpler than dbt."

**What impresses:**
DuckDB is bundled -- no system installation required. The project setup is straightforward: create a `smelt.yml`, put SQL files in `models/`, use `smelt.ref('other_model')` for dependencies, and run `smelt run`. The `smelt seed` command loads CSVs, and `smelt build` does seed + run in one step. The `smelt ui` command launches a web dashboard showing the model graph visually. For a small DuckDB project, this is a clean, self-contained experience.

The type checking catches mistakes before execution. If you mistype a column name or use the wrong type in a JOIN condition, the LSP tells you immediately.

**What concerns:**
Installation requires either building from source (Rust toolchain) or finding a pre-built binary release. There is no `pip install smelt` or `brew install smelt`. The documentation is developer-focused, not analyst-focused -- there is no "5-minute quickstart for non-programmers" guide. Error messages from Rust panics (and there are 935 `unwrap()` calls in the codebase) could be cryptic. There is no community to ask questions in.

**Verdict: Cautious Adopt**

For a DuckDB-only local project, smelt works today and offers advantages over raw SQL scripts or a full dbt setup. The LSP and type checking are luxuries analysts rarely get. Just be prepared for some rough edges.

**Recommendations:**
1. Ensure pre-built binaries are available on GitHub Releases for Linux, macOS, and Windows
2. Write a "Getting Started in 5 Minutes" guide targeting non-Rust users
3. Reduce `unwrap()` calls in user-facing code paths to improve error messages

---

## Perspective 8: Senior Rust Architect

> "I'm evaluating the system design, crate architecture, and long-term maintainability of this Rust codebase."

**What impresses:**
The crate architecture follows best practices. `smelt-parser` is standalone (no Salsa dependency) and can be used by any consumer -- LSP, CLI, planner, or future tools. `smelt-db` wraps it with Salsa for incremental computation. `smelt-dialect` is lightweight and avoids linking heavy dependencies (Arrow, Tokio, DuckDB), keeping the LSP binary lean. This layering is the same pattern used by rust-analyzer and is the gold standard for compiler-like tools.

The Rowan + Salsa combination delivers lossless syntax trees with automatic fine-grained dependency tracking. The parser's error recovery produces ERROR nodes rather than panicking, enabling partial analysis of incomplete code. The property-based testing infrastructure (generators, oracles, divergence registry in `crates/smelt-db/tests/prop_helpers/`) is mature. The fuzz targets (`parse_never_panics`, `round_trip`) in `crates/smelt-parser/fuzz/` verify parser robustness. There are only 4 `unsafe` blocks in the entire codebase.

**What concerns:**
There are 935 `unwrap()` calls across the codebase. The CLI (`crates/smelt-cli/src/main.rs` at 2,387 lines) accounts for many of these. This is the primary reliability risk -- any unexpected `None` or `Err` in a user-facing code path produces a panic with a Rust backtrace instead of a helpful error message.

The CLI `main.rs` at 2,387 lines mixes argument parsing, execution orchestration, and output formatting. This should be decomposed into subcommand modules. There are 320 `println!` calls and only 8 `tracing::` calls -- the project lacks structured logging entirely. Salsa 0.16 is used rather than the newer 0.18+ which has a different (improved) API and better performance characteristics.

**Verdict: Strong foundation, needs hardening**

The architecture is sound and demonstrates mastery of Rust's compiler tooling ecosystem. The main technical debt items (unwrap proliferation, missing structured logging, oversized main.rs) are addressable without architectural changes.

**Recommendations:**
1. Introduce `tracing` across all crates with structured spans for the compilation pipeline
2. Systematic `unwrap()` audit: replace with `anyhow::Context` in CLI, `thiserror` variants in libraries
3. Decompose `main.rs` into per-subcommand modules
4. Evaluate Salsa 0.18+ migration (breaking API change but improved architecture)

---

## Perspective 9: Senior Rust Developer

> "I'm considering contributing to this project. What's the code quality and contribution experience like?"

**What impresses:**
CI enforces `cargo clippy -- -D warnings` and `cargo fmt --all -- --check`. The codebase compiles cleanly with no warnings. Error types use `thiserror` consistently in library crates (`BackendError`, `TypeParseError`, `CliError`) with descriptive variants. Domain modeling uses Rust's type system well: `Materialization` enum, `BatchSafety` enum, `PhysicalStrategy` enum, `IncrementalConfig` struct -- these make illegal states unrepresentable.

Test coverage is strong: 919 test functions across 16 crates, with the parser (226 tests), database layer (118 tests), and CLI (115 tests) being the most thoroughly tested. The `CLAUDE.md` file provides comprehensive onboarding context including build commands, architecture overview, and development workflow.

**What concerns:**
There is no `CONTRIBUTING.md` -- no guide for new contributors, no PR process, no code style expectations beyond what clippy enforces. Error handling is inconsistent: some modules use `anyhow::Result`, others use custom error types, and many use `unwrap()`. The `crates/smelt-cli/src/python.rs` module (Python model extraction) has particularly heavy `unwrap()` usage and is the most fragile module. There are no snapshot tests (insta crate) for compiler SQL output, which would be the most natural regression test for a compiler project.

**Verdict: Good contribution experience**

The codebase is well-organized, Rust idioms are followed, and the crate structure provides natural boundaries for understanding the system. A contributing guide and snapshot tests would significantly improve the onboarding experience.

**Recommendations:**
1. Add `CONTRIBUTING.md` with build instructions, architecture overview, and PR guidelines
2. Add snapshot tests (insta crate) for compiler SQL output across dialects
3. Standardize error handling: `thiserror` for library crates, `anyhow` for CLI, zero `unwrap()` in user-facing paths

---

## Perspective 10: Senior Python Developer

> "I want to understand the Python integration story. Can I extend smelt from Python?"

**What impresses:**
The `@model` decorator pattern in `python/smelt/core.py` is clean -- write a function that returns SQL, tag it with `@model`, and smelt discovers it. The planner rules SDK (`python/smelt-sdk/`) provides a `PlannerRule` base class with typed data models (`ModelInfo`, `Opportunity`, `Transformation`). Built-in Python rules for cube split and incremental detection in `python/smelt-rules-builtin/` demonstrate the extension pattern. PyO3 bridges Rust planner with Python rules.

**What concerns:**
The Python SDK is on TestPyPI only -- it cannot be `pip install`ed from the real PyPI. Python model execution works via subprocess: the Rust CLI spawns a Python process, passes project context as JSON, and parses the stdout JSON response. This is fragile (125 `unwrap()` calls in `crates/smelt-cli/src/python.rs`), has no streaming, and any Python error could crash the Rust process with an unhelpful panic.

There are no Python type stubs (`.pyi` files) for IDE support, no pytest fixtures for testing Python models, and the `ProjectContext` API is minimal -- only `find_models()`, no access to schemas, types, or configs. There is no integration with PySpark, pandas, or polars for native Python data manipulation.

**Verdict: Wait**

The Python integration is a proof of concept, not production-ready. The SDK type design is good, but the execution bridge (subprocess + stdout parsing + 125 unwraps) is the most fragile part of the entire codebase.

**Recommendations:**
1. Publish the SDK to real PyPI
2. Replace subprocess execution with direct PyO3 embedding (partially implemented, needs completion)
3. Add Python type stubs and pytest fixtures for the extension API
4. Handle Python errors gracefully -- never panic on Python subprocess failures

---

## Cross-Cutting Themes

### Theme 1: The Testing Gap is the Universal Blocker

Every production-oriented perspective (dbt user, Director of Engineering, SQLMesh user, Analytics Engineer) flagged the absence of a data testing framework. The codebase itself is well-tested (919 tests, property-based testing, fuzzing), but there is no mechanism for users to validate their *data*. This is the single highest-impact feature to add.

### Theme 2: DuckDB Island

smelt works well for DuckDB workflows. Every other backend is either stub (Spark: 267 LOC, all errors) or absent (PostgreSQL, BigQuery, Snowflake, Redshift). The multi-backend vision is architecturally supported by the clean Backend trait and dialect system, but practically, this is a DuckDB tool today.

### Theme 3: Impressive Engineering, Missing Ecosystem

The parser, type system, LSP, and incremental analysis are genuinely innovative -- arguably best-in-class for data transformation tools. But tools succeed on ecosystem, not just technical merit. dbt's package system, community, and integrations are a major moat. smelt has zero packages, zero community plugins, and zero third-party integrations.

### Theme 4: Observability and Production Readiness

320 `println!` calls and 8 `tracing::` calls tell the story. There is no structured logging, no metrics export, no health checks, and no monitoring hooks. Combined with 935 `unwrap()` calls, the operational story for production deployment is not ready.

### Theme 5: Single Author Risk

Every organizational perspective flagged the bus factor. The clean architecture and thorough CLAUDE.md mitigate this somewhat -- a competent Rust developer could understand the system. But there is no CONTRIBUTING.md, no community governance, and no second maintainer.

---

## Summary Matrix

| Perspective | Verdict | Top Strength | Top Concern | Priority Recommendation |
|---|---|---|---|---|
| dbt User | **Wait** | Type-safe SQL, no Jinja | No data testing framework | Add `smelt test` |
| Director of Engineering | **Pass** | Engineering quality, CI | Bus factor of 1 | Wait for community growth |
| SQLMesh User | **Pass** | Parser quality, temporal analysis | No virtual environments | Stay on SQLMesh |
| Senior Data Architect | **Wait** | Logical/physical separation | No lineage export | Add OpenLineage |
| Senior Analytics Engineer | **Adopt (DuckDB)** | LSP productivity | No reusable SQL patterns | Publish editor guides |
| Data Engineer (Spark) | **Hard Pass** | Backend trait design | Spark is a non-functional stub | Be transparent about status |
| Data Analyst | **Cautious Adopt** | Zero-dep bundled DuckDB | Installation friction | Publish pre-built binaries |
| Senior Rust Architect | **Strong Foundation** | Rowan + Salsa architecture | 935 unwrap() calls | Add tracing, fix unwraps |
| Senior Rust Developer | **Good** | Clean crate structure | No CONTRIBUTING.md | Add contribution guide |
| Senior Python Developer | **Wait** | SDK type design | TestPyPI only, fragile bridge | Publish to PyPI |

---

## Prioritized Recommendations

### Quick Wins (< 1 week each)

1. **Add CONTRIBUTING.md** with build instructions, architecture overview, and contribution guidelines
2. **Publish pre-built binaries** via GitHub Releases (dev-release.yml workflow exists)
3. **Publish Python SDK to PyPI** (not just TestPyPI)
4. **Write a dbt-to-smelt cheat sheet** showing common pattern equivalents
5. **Update documentation to be transparent about Spark status** -- distinguish "designed for" from "implemented"

### High Impact (1-4 weeks each)

1. **Implement `smelt test`** with schema-level assertions (not_null, unique, relationships, custom SQL)
2. **Replace `println!` with `tracing`** across all crates, with structured spans for the compilation pipeline
3. **Audit and fix `unwrap()` calls** in user-facing code, especially `python.rs` (125 unwraps) and `main.rs`
4. **Add snapshot tests** (insta crate) for compiler SQL output across all supported dialects
5. **Decompose CLI `main.rs`** (2,387 lines) into per-subcommand modules

### Strategic (1-3 months each)

1. **Implement `smelt docs generate`** for data catalog / data dictionary output
2. **Build PostgreSQL backend** as the second production-ready backend
3. **Add virtual environment / plan-apply workflow** for safe schema change management
4. **Implement OpenLineage export** for data catalog integration
5. **Build a package/dependency system** for reusable model libraries
6. **Attract a second maintainer** to reduce bus factor

---

## Appendix: Codebase Statistics

### Lines of Code by Crate

| Crate | LOC | Tests | Purpose |
|---|---|---|---|
| smelt-cli | 12,320 | 115 | CLI entry point, orchestration, graph building |
| smelt-db | 9,154 | 118 | Salsa incremental queries, type inference |
| smelt-parser | 7,956 | 226 | Rowan CST parser, lexer, AST |
| smelt-parser-compat | 3,775 | 148 | Cross-dialect conformance testing |
| smelt-core | 3,723 | 81 | Project discovery, config, dependency graphs |
| smelt-lsp | 3,270 | 51 | Language Server Protocol implementation |
| smelt-planner | 2,938 | 63 | Temporal analysis, cube split, optimization rules |
| smelt-datagen | 2,385 | 8 | Test data generation |
| smelt-ui | 2,382 | 5 | Axum web dashboard with embedded React |
| smelt-bench | 2,017 | 12 | Performance benchmarks |
| smelt-state | 1,468 | 32 | Run manifests, interval tracking |
| smelt-types | 1,390 | 21 | Core data type definitions |
| smelt-dialect | 996 | 39 | Multi-dialect SQL printer |
| smelt-backend-duckdb | 616 | 0 | DuckDB execution backend |
| smelt-backend | 436 | 0 | Backend trait definition |
| smelt-backend-spark | 267 | 0 | Spark stub (non-functional) |
| **Total** | **55,093** | **919** | |

### Key Dependencies

| Dependency | Version | Purpose |
|---|---|---|
| rowan | 0.15 | Lossless CST representation |
| salsa | 0.16 | Incremental computation framework |
| datafusion | 43 | SQL type coercion rules (validation only) |
| duckdb | 1.4.4 | Execution backend + test oracle |
| arrow | 57 | Data interchange format |
| tower-lsp | 0.20 | LSP protocol implementation |
| tokio | 1 | Async runtime |
| thiserror | 2.0 | Structured error types |
| anyhow | 1.0 | Error context propagation |
| proptest | 1.4 | Property-based testing |
| pyo3 | 0.24 | Python embedding |

### Quality Metrics

| Metric | Value |
|---|---|
| Total Rust LOC | 55,093 |
| Test functions | 919 |
| Fuzz targets | 2 |
| CI workflows | 9 |
| `unsafe` blocks | 4 |
| `unwrap()` calls | 935 |
| `println!` calls | 320 |
| `tracing::` calls | 8 |
| Clippy warnings | 0 (enforced in CI) |
