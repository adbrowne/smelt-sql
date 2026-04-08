# smelt Codebase Review Report

**Date**: April 9, 2026
**Version reviewed**: 0.1.0
**Codebase**: ~85,000 LOC Rust, 16 crates, 1,504 tests
**Previous review**: [March 26, 2026](codebase-review-2026-03-26.md) (~55,000 LOC, 919 tests)
**Review methodology**: Multi-perspective analysis from 10 professional viewpoints
**Author**: Independent review

---

## Executive Summary

Two weeks after the initial review, smelt has addressed the majority of its critical gaps while growing 54% in codebase size. The project has executed an impressive sprint that resolved the top-priority recommendation from every production-oriented perspective: a data testing framework (`smelt test`) now exists with whole-model, CTE-level, and property-based testing. The Spark backend has gone from a non-functional stub to a working PyO3/PySpark bridge. Schema evolution, documentation generation, seeds, and LSP refactoring features have all shipped.

**Top 3 strengths:**
1. **Rapid, focused execution**: 7 of 11 "quick win" and "high impact" items from the March review have been addressed in two weeks
2. **Feature completeness**: smelt now has a complete data transformation workflow -- models, sources, seeds, tests, incremental execution, schema evolution, docs generation, and a web UI
3. **LSP is best-in-class**: With the addition of rename refactoring, find references, and code actions, smelt's LSP now offers capabilities no other data transformation tool provides

**Top 3 risks:**
1. **Bus factor of 1**: Still a single-author project with no external contributors or community governance
2. **Backend coverage**: DuckDB is production-grade, Spark is functional but new and untested at scale; PostgreSQL, Snowflake, and BigQuery are absent
3. **unwrap() growth**: While the ratio improved (1.7% of LOC vs 1.7% previously), absolute count grew from 935 to 1,157, with `python.rs` still at 125 unchanged

**Verdict shift**: Several perspectives have upgraded their assessments. The Spark perspective moves from "Hard Pass" to "Cautious Evaluate." The dbt user perspective moves from "Wait" to "Cautious Adopt." The Director of Engineering perspective moves from "Pass" to "Wait with Interest."

---

## Progress Since Previous Review

### Previous Recommendations Scorecard

#### Quick Wins (< 1 week each)

| # | Recommendation | Status | Notes |
|---|---|---|---|
| 1 | Add CONTRIBUTING.md | **Done** | `docs-site/docs/developing/contributing.md` -- 193 lines covering build, test, code quality, workflow |
| 2 | Publish pre-built binaries | **Done** | `release.yml` builds Linux x86_64/aarch64, macOS aarch64, Windows x86_64. `pip install smelt-sql` also works |
| 3 | Publish Python SDK to PyPI | **Not done** | SDK still local-only. `smelt-sql` CLI is on PyPI but the planner rules SDK is not |
| 4 | Write a dbt-to-smelt cheat sheet | **Not done** | Only comparative mentions in README and docs, no dedicated migration guide |
| 5 | Update docs re: Spark status | **Resolved differently** | Spark is now functional, so the transparency concern is largely moot. README claims "full" which is slightly ahead of reality but not misleading |

#### High Impact (1-4 weeks each)

| # | Recommendation | Status | Notes |
|---|---|---|---|
| 1 | Implement `smelt test` | **Done** | Full framework: whole-model tests, CTE tests, property-based tests, SQL body tests, reproducible seeds |
| 2 | Replace `println!` with `tracing` | **In progress** | println! reduced 320->220 (-31%), tracing increased 8->32 (+300%). Trend is correct but not complete |
| 3 | Audit and fix `unwrap()` calls | **Partially done** | Overall ratio stable despite 54% codebase growth. `python.rs` still has 125 unwraps (unchanged). New code in smelt-state uses unsafe for Spark DDL |
| 4 | Add snapshot tests (insta) | **Done** | 30 `assert_snapshot!` tests in `smelt-dialect` for cross-dialect SQL output verification |
| 5 | Decompose CLI `main.rs` | **Done** | 2,387 lines -> 414 lines. Decomposed into `commands/` module with 14 subcommand files |

#### Strategic (1-3 months each)

| # | Recommendation | Status | Notes |
|---|---|---|---|
| 1 | Implement `smelt docs generate` | **Done** | Markdown and JSON output, selective generation, model descriptions |
| 2 | Build PostgreSQL backend | **Not done** | `SqlDialect::PostgreSQL` enum exists but no backend crate |
| 3 | Add virtual environments / plan-apply | **Not done** | No schema comparison or approval workflow |
| 4 | Implement OpenLineage export | **Not done** | No lineage export API |
| 5 | Build a package system | **Not done** | No reusable model libraries |
| 6 | Attract a second maintainer | **Not done** | Still single-author |

**Summary: 7 of 16 recommendations fully addressed, 2 partially addressed, 7 not addressed.** The team prioritized correctly -- the testing framework, CLI decomposition, and snapshot tests were the highest-impact items.

### Additional Improvements Not in Previous Recommendations

These features shipped since the March review but were not explicitly recommended:

- **Schema evolution**: Full ALTER TABLE migration path with column add/remove/type-change support, safety flags, and Spark-specific table rewrite handling
- **Seeds support**: CSV-based seed loading with source/target distinction, selective loading, and `smelt build` integration
- **LSP rename refactoring**: Cross-file rename with prepare-rename support
- **LSP find references**: Downstream consumer discovery for models, sources, and CTEs
- **LSP code actions**: CAST suggestions, create model, add source/column, extract/inline CTE
- **smelt-state crate**: Run manifests, interval tracking, schema tracking, file-based state persistence
- **Documentation site**: 24-page MkDocs Material site deployed at smeltsql.com
- **`smelt diff` command**: Offline schema change detection with risk assessment
- **`smelt history` command**: Run history with configurable limits
- **`smelt backbuild` command**: Rebuild model + upstreams for time range with batch/per-partition modes

---

## Perspective 1: Current dbt User Considering a Switch

> "I've been running dbt for three years. My project has 200+ models, custom macros, and a test suite. What does smelt offer me?"

**What impresses:**

The March review's #1 blocker -- the absence of a data testing framework -- has been resolved. `smelt test` now supports whole-model tests (mock all dependencies, assert full output), CTE-level tests (isolate intermediate transformations), property-based tests (randomized inputs with configurable case counts), and advanced SQL body tests for complex mock data. Tests are co-located with models or placed in a `tests/` directory. The comparison engine handles set vs. ordered comparison, column filtering, numeric tolerance, and type coercion. CI integration is straightforward: exit code 0 for all pass, 1 for any failure.

The elimination of Jinja remains the core value proposition. Models are pure SQL with YAML frontmatter. Schema evolution is now automatic -- column additions generate ALTER TABLE statements, type changes are handled per-backend, and dangerous operations (column removal, full refresh) require explicit flags. The `smelt diff` command shows pending schema changes before execution, providing a dbt-equivalent of `dbt run --defer`.

Seeds support has landed, replacing the need for `dbt seed`. CSV files in `seeds/` are auto-discovered with source/target distinction (`seeds/<source>/<table>.csv` for source seeds).

**What concerns:**

There is still no macro or reusable SQL pattern system. dbt macros are messy but they solve a real need -- date spine generation, surrogate key hashing, standard metric calculations. Python models can generate SQL dynamically, but this requires switching languages. There is no package ecosystem (dbt-utils, dbt-expectations, dbt-date). There are no snapshots/SCD Type 2. There is no `dbt docs serve`-style interactive data catalog -- `smelt docs generate` outputs markdown or JSON but doesn't serve an explorable UI.

The web UI (`smelt ui`) shows a model graph but lacks the documentation integration that makes dbt's docs site useful for cross-team discovery.

**Verdict: Cautious Adopt** (upgraded from Wait)

For DuckDB-based projects, smelt now has feature parity with dbt Core on the core workflow: models, sources, seeds, tests, incremental execution, and documentation. The LSP and type system provide genuine advantages dbt cannot match. The main gaps are ecosystem (no packages) and workflow (no snapshots, no interactive docs).

**Recommendations:**
1. Create a dbt-to-smelt migration guide with pattern equivalents (Jinja -> frontmatter, macros -> Python models, tests -> smelt test)
2. Add `smelt docs serve` for an interactive data catalog (or integrate docs into `smelt ui`)
3. Consider a lightweight SQL reuse mechanism (parameterized includes or SQL templates)

---

## Perspective 2: Director of Engineering Considering Adoption

> "One of my teams wants to adopt this. I need to understand the risk profile."

**What impresses:**

The execution velocity since the March review is remarkable. In two weeks, the project shipped a testing framework, schema evolution, seeds, CLI decomposition, documentation site, 3 new LSP features, and a functional Spark backend. The codebase grew 54% while maintaining zero clippy warnings and expanding test coverage by 64%. This suggests a disciplined developer with strong architectural foundations that enable rapid feature delivery.

The documentation story has improved dramatically. A 24-page MkDocs site at smeltsql.com covers getting started, concepts, guides, and reference. CONTRIBUTING.md exists. CI enforces formatting and linting. The release workflow builds binaries for 5 platforms and publishes to PyPI.

**What concerns:**

This is still a single-author project. The impressive velocity is also the risk -- it means one person's availability determines the project's future. There are no external contributors, no community channels, no governance structure, and no known production deployments beyond the author's own use.

The Spark backend is now functional via PyO3/PySpark, but it's brand new and has no test cases in the backend crate itself (relying on integration tests). Deploying this to a production Spark cluster would require careful validation. There is no orchestrator integration (Airflow, Dagster, Prefect), no structured credential management, and no audit logging.

The `unsafe` block count increased from 4 to 12, with 8 new blocks in `smelt-state/src/schema_tracking.rs` related to Spark DDL handling. These deserve scrutiny in a security review.

**Verdict: Wait with Interest** (upgraded from Pass)

The feature gaps that previously made this a "pass" are closing rapidly. The testing framework, schema evolution, and documentation site address the most critical operational concerns. However, the bus factor and lack of production deployments remain blockers for organizational adoption.

**Recommendations:**
1. If seriously interested, run a time-boxed pilot on a non-critical DuckDB analytics project
2. Engage the author about roadmap, support model, and community-building plans
3. Require a security review of the 12 `unsafe` blocks before any production deployment
4. Wait for at least one documented production deployment before broader adoption

---

## Perspective 3: Current SQLMesh User Considering a Switch

> "I use SQLMesh for its virtual environments and plan/apply workflow. What does smelt do better?"

**What impresses:**

The `smelt diff` command provides a rudimentary plan/apply-style workflow -- it shows pending schema changes (column adds, type changes, removals) with risk assessment before execution. This is not as sophisticated as SQLMesh's full virtual environment comparison, but it addresses the core need of "show me what will change before I change it."

The testing framework brings smelt to parity with SQLMesh's test capabilities. Property-based testing with randomized inputs is a capability SQLMesh lacks. The schema evolution system with ALTER TABLE migrations is also competitive -- SQLMesh relies on its virtual layer for schema management rather than direct DDL.

The LSP improvements (rename, references, code actions) widen the developer experience gap further in smelt's favor.

**What concerns:**

SQLMesh's two killer features remain absent. Virtual environments (comparing schemas across dev/prod without materializing) do not exist. The plan/apply workflow (`smelt diff` shows changes but doesn't provide an approval gate or rollback mechanism). Column-level lineage tracking is still internal only with no export API.

SQLMesh supports BigQuery, Snowflake, Databricks, Postgres, and more. smelt has DuckDB (production) and Spark (new, unproven at scale). For multi-cloud data teams, backend coverage is decisive.

**Verdict: Pass** (unchanged)

SQLMesh remains more mature in production workflow features. smelt's LSP, type system, and testing are technically stronger, but virtual environments and backend coverage are hard requirements for teams already on SQLMesh.

**Recommendations:**
1. Implement a formal plan/apply workflow with approval gates and rollback
2. Add column-level lineage export (OpenLineage format)
3. The LSP remains smelt's strongest differentiator -- continue investing here

---

## Perspective 4: Senior Data Architect

> "I'm evaluating the system design for scalability, integration patterns, and long-term viability."

**What impresses:**

The new `smelt-state` crate (6,747 LOC) demonstrates architectural maturity. It provides run manifests (tracking execution metadata per model), interval tracking (what time ranges have been processed), schema tracking (deployed vs. expected schemas with diff computation), and file-based state persistence. The schema tracking module at 3,539 lines handles column-level diffing, migration planning, and backend-specific DDL generation. This is the infrastructure needed for production-grade state management.

Schema evolution is now comprehensive. `SchemaEvolutionResult` covers first deployment, no change, migrated (with ALTER TABLE statements), full refresh required (with reason), column removal blocked (requires flag), and Spark-specific table rewrites. The migration system respects backend capabilities -- DuckDB gets ALTER COLUMN TYPE while Spark may require table rewrite for Parquet type changes.

The `smelt diff` command enables offline schema change assessment, which is architecturally sound for CI/CD integration.

**What concerns:**

No data lineage export exists. The type system tracks column provenance internally, but there is no API to export to catalog tools (DataHub, Amundsen, Atlan) or OpenLineage format. The DuckDB incremental strategy concern from the March review (non-atomic DELETE + INSERT) has not been explicitly addressed -- it's unclear if this is now wrapped in a transaction.

Multi-backend routing within a single run is architecturally supported (per-model `target` override in frontmatter) but the cross-engine data transfer mechanism relies on Parquet files. There is no Substrait integration for portable query plans.

**Verdict: Wait with Strong Interest** (unchanged, but stronger)

The architecture continues to impress. The smelt-state crate, schema evolution system, and CLI decomposition show the project is maturing toward production-grade infrastructure. The logical/physical separation and backend abstraction remain well-designed.

**Recommendations:**
1. Implement OpenLineage export for catalog integration
2. Ensure DuckDB incremental operations are transactional (wrap DELETE+INSERT in a transaction)
3. Add a lineage API that exposes column-level provenance from the type system
4. Document the cross-engine data transfer mechanism (Parquet exchange) for architects evaluating multi-backend scenarios

---

## Perspective 5: Senior Analytics Engineer

> "I write 5-10 models a week and care about productivity. Will smelt make me faster?"

**What impresses:**

The LSP has gained three major features since the March review. Rename refactoring works across files -- rename a model and all `smelt.ref()` calls update. Find references shows all downstream consumers of a model, source, or CTE. Code actions offer CAST suggestions when types don't match, create new model files, add sources/columns to YAML, and extract/inline CTEs. These are IDE features that analytics engineers have never had for SQL transformation work.

The testing framework enables a tight feedback loop: write a model, write a co-located test, run `smelt test`, see results with timing. Property-based tests catch edge cases that manual test data misses. The `smelt diff` command previews schema changes before execution.

Seeds support means the full analytics workflow (load reference data, transform, test, document) happens within smelt without external tools.

**What concerns:**

There is still no reusable SQL pattern mechanism. The same date spine logic, surrogate key generation, or standard metric calculation must be copy-pasted or wrapped in Python models. The documentation site exists but `smelt docs generate` produces static markdown/JSON -- there's no interactive explorer. The web UI graph visualization is useful but lacks documentation integration.

Editor support beyond VSCode requires manual LSP configuration. No Neovim, Emacs, or JetBrains plugins exist.

**Verdict: Adopt for DuckDB projects** (unchanged, but stronger)

The LSP improvements make this an even stronger recommendation. For DuckDB-based analytics, smelt now provides a complete workflow with productivity advantages no other tool matches.

**Recommendations:**
1. Add SQL template/include mechanism for reusable patterns
2. Integrate documentation into `smelt ui` for a browsable data catalog
3. Publish generic LSP configuration examples for Neovim, Emacs, and Helix

---

## Perspective 6: Senior Data Engineer (PySpark / Scala Spark)

> "My team runs 2,000+ Spark jobs daily on Databricks. I need reliable Spark integration."

**What impresses:**

The Spark backend has undergone a fundamental transformation. Where the March review found 267 lines of error-returning stubs, there is now a functional PyO3-based PySpark bridge. The implementation uses in-process Python execution (not subprocess), supports both Spark Connect (`sc://localhost:15002`) and Databricks Connect, handles qualified names (`catalog.schema.table`), and implements MERGE, DELETE partitions, INSERT OVERWRITE, and standard CREATE/DROP operations. The `smelt-state` crate includes Spark-specific DDL generation (`ddl_spark.rs`) and schema evolution handles Spark's Parquet type change limitations (requiring table rewrite instead of ALTER COLUMN TYPE).

The `smelt-backend-spark` crate has grown from 267 to 360 lines of actual implementation plus 926 lines of supporting infrastructure in `smelt-state`. The cross-engine architecture allows Parquet-based data exchange between DuckDB and Spark within a single pipeline.

**What concerns:**

The Spark backend has **zero test cases** in the backend crate itself. It relies entirely on integration tests that require a running Spark/Databricks cluster. There is no CI validation of Spark functionality -- the test suite only tests DuckDB. The `unsafe impl Send` and `unsafe impl Sync` on `SparkBackend` are necessary for the PyO3 bridge but require careful reasoning about GIL safety.

There is no Delta Lake table format support (Spark writes Parquet), no partitioning strategy configuration (Spark's `PARTITIONED BY`), no resource management (executor memory, cores), and no Spark-specific monitoring. The implementation is functional but untested at scale.

The Python model execution for Spark also lacks streaming support and any PySpark DataFrame integration -- models generate SQL strings, not Spark DataFrames.

**Verdict: Cautious Evaluate** (upgraded from Hard Pass)

The Spark backend now exists and is architecturally sound. For teams willing to be early adopters and validate against their specific Spark environment, there is enough here to evaluate. However, the lack of any automated Spark testing means production deployment carries risk.

**Recommendations:**
1. Add integration test infrastructure for Spark (Docker Compose with Spark standalone or Databricks Connect mock)
2. Add Delta Lake table format support (Delta is the standard for Databricks)
3. Document the GIL safety reasoning for `unsafe impl Send/Sync` on SparkBackend
4. Add Spark-specific examples demonstrating partition management and large-scale patterns

---

## Perspective 7: Data Analyst Maintaining a Small Project

> "I have 15 models that transform CSVs into dashboards using DuckDB. I want something simpler than dbt."

**What impresses:**

The onboarding experience has improved substantially. `pip install smelt-sql` provides the CLI and LSP server as native binaries -- no Rust toolchain required. The documentation site at smeltsql.com has a quickstart guide, installation instructions for 3 methods (pip, standalone binaries, source build), and dedicated guides for SQL models, sources, seeds, testing, and editor setup.

The workflow is now complete: put CSVs in `seeds/`, write SQL models in `models/`, write tests in `tests/`, run `smelt build` (seeds + models), run `smelt test`, generate docs with `smelt docs generate`. The web UI (`smelt ui`) shows the model graph visually. Type checking catches mistakes before execution.

Schema evolution means adding a column to a model doesn't require manually dropping and recreating tables.

**What concerns:**

Error messages from Rust panics remain a risk. While the CLI has been decomposed and error handling has improved, `python.rs` still has 125 `unwrap()` calls that could produce cryptic backtraces if a Python model fails unexpectedly. The overall `unwrap()` count (1,157) means unexpected failures in edge cases will produce developer-facing error messages rather than user-friendly ones.

There is no community to ask questions in -- no Discord, Slack, or forum. The documentation is good but there are no video tutorials, blog posts, or "cookbook" examples for common analyst tasks.

**Verdict: Adopt** (upgraded from Cautious Adopt)

For a DuckDB-only local project, smelt is now a complete, well-documented tool that offers significant advantages over raw SQL scripts or a full dbt setup. The pip installation, documentation site, and testing framework address the previous friction points.

**Recommendations:**
1. Create a "Cookbook" page with common analyst patterns (date spine, running totals, cohort analysis)
2. Continue reducing `unwrap()` calls in user-facing code paths, especially `python.rs`
3. Consider setting up a GitHub Discussions or Discord for community questions

---

## Perspective 8: Senior Rust Architect

> "I'm evaluating the system design, crate architecture, and long-term maintainability of this Rust codebase."

**What impresses:**

The CLI decomposition is textbook good refactoring. The 2,387-line `main.rs` has been split into a 414-line entry point plus 14 subcommand modules under `commands/` (run, build, backbuild, seed, test, docs, explain, diff, status, history, table, type, ui). Helper modules handle cross-cutting concerns: `compiler.rs`, `executor.rs`, `selector.rs`, `temporal.rs`, `migration.rs`. This is the correct architecture for a CLI of this complexity.

The `smelt-state` crate (6,747 LOC, 189 tests) is a well-designed addition. It cleanly separates state management from execution, with backend-specific DDL generation (`ddl_duckdb.rs`, `ddl_spark.rs`), schema tracking with diff computation, and file-based persistence. The separation means state can be tested independently of backends.

The snapshot tests via insta (30 assertions in `smelt-dialect`) provide the regression safety that a compiler project needs. The property-based test infrastructure continues to mature.

**What concerns:**

The `unwrap()` count has grown from 935 to 1,157. While the ratio to total LOC has remained stable (~1.4%), the absolute count means more potential panic points. The `python.rs` module at 125 unchanged unwraps remains the most fragile user-facing code path.

The `unsafe` block count has tripled from 4 to 12. The 8 new blocks in `smelt-state/src/schema_tracking.rs` warrant explanation -- this is a state management module that shouldn't typically need unsafe code. The `Send`/`Sync` implementations on `SparkBackend` are a common pattern for PyO3 but carry GIL-safety obligations.

Salsa remains on 0.16 while 0.18+ offers improved APIs and performance. This is not urgent but will become increasingly expensive to defer as the codebase grows.

The LSP `lib.rs` at 4,116 lines is large for a single file but not egregious -- this was reported as 176K LOC by one agent, which appears to be an error. The actual 4,116 lines are manageable but would benefit from module extraction.

**Verdict: Strong foundation, maturing well** (improved from "needs hardening")

The CLI decomposition and smelt-state crate demonstrate architectural discipline. The project is making the right structural investments. The remaining concerns (unwrap audit, unsafe audit, Salsa upgrade) are addressable without architectural changes.

**Recommendations:**
1. Audit the 8 `unsafe` blocks in `schema_tracking.rs` -- document why unsafe is needed or refactor to safe alternatives
2. Systematic `unwrap()` reduction in `python.rs` (125 calls) -- this is the highest-risk user-facing module
3. Plan for Salsa 0.18+ migration before the codebase doubles again
4. Split LSP `lib.rs` (4,116 lines) into diagnostic, hover, completion, reference, and action modules

---

## Perspective 9: Senior Rust Developer

> "I'm considering contributing to this project. What's the code quality and contribution experience like?"

**What impresses:**

The contribution path is now documented. `docs-site/docs/developing/contributing.md` covers prerequisites (Rust, Node.js, Python), build instructions, code quality expectations (cargo fmt + clippy), testing strategy, and development workflow. The architecture documentation at `docs-site/docs/developing/architecture.md` provides system design context.

CI remains strict: `cargo clippy -- -D warnings` and `cargo fmt --all -- --check` are enforced. The test suite has grown from 919 to 1,504 test functions -- a 64% increase that outpaces the 54% codebase growth, indicating improving test discipline. Snapshot tests via insta provide easy-to-maintain regression coverage for the dialect printer.

The crate structure provides clear module boundaries. A contributor interested in the parser can work in `smelt-parser` without understanding Salsa. A contributor interested in Spark can work in `smelt-backend-spark` without understanding the parser. The 14 CLI subcommand files make it easy to find and modify specific command behavior.

**What concerns:**

There is still no `CODE_OF_CONDUCT.md`, no issue templates, and no PR templates. These are table-stakes for open-source projects seeking contributors. The git workflow documented in CONTRIBUTING.md says "for small changes, commit directly to main" -- this is fine for a single author but needs to change for multi-contributor projects.

Error handling remains inconsistent. Some modules use `anyhow::Result` with `.with_context()`, others use custom error types via `thiserror`, and many use `unwrap()`. The `commands/` directory shows good error handling patterns (the new code is better), but older modules like `python.rs` have not been updated.

There are no "good first issue" labels, no contributor onboarding issues, and no community presence beyond the GitHub repository.

**Verdict: Good contribution experience** (unchanged)

The codebase is well-organized and the contributing guide exists. The main barrier to contributions is community infrastructure (issue templates, labels, communication channels) rather than code quality.

**Recommendations:**
1. Add `CODE_OF_CONDUCT.md`, issue templates, and PR templates
2. Create "good first issue" labels for straightforward tasks (error handling cleanup, documentation, test additions)
3. Set up GitHub Discussions or Discord for contributor communication
4. Standardize error handling: document when to use `anyhow` vs `thiserror` vs propagation

---

## Perspective 10: Senior Python Developer

> "I want to understand the Python integration story. Can I extend smelt from Python?"

**What impresses:**

The Python model execution has moved from subprocess to in-process PyO3 embedding. This is a significant architectural improvement -- Python models execute within the Rust process via `pyo3::Python::with_gil()`, eliminating subprocess overhead, enabling shared state, and providing better error propagation. The `@model` decorator pattern is clean, and the `ProjectContext` API supports model discovery with tag and directory filtering.

The Spark backend uses the same PyO3 bridge, meaning PySpark sessions are managed within the Rust process. This enables the "write SQL in smelt, execute on Spark" workflow without subprocess overhead.

The planner rules SDK (`python/smelt-sdk/`) provides typed data models (`ModelInfo`, `Opportunity`, `Transformation`) for writing custom optimization rules. Built-in Python rules for cube split and incremental detection demonstrate the extension pattern.

The documentation at `docs-site/docs/guide/python-models.md` (128 lines) covers when to use Python models, the decorator API, project context, multiple models per file, helper functions, and Python interpreter configuration.

**What concerns:**

The Python SDK is still not published to PyPI. `pip install smelt-sql` installs the CLI binary, but the planner rules SDK requires source installation. There are no Python type stubs (`.pyi` files) for IDE support, no pytest fixtures for testing Python models, and no integration with pandas, polars, or PySpark DataFrames.

The `python.rs` module in the CLI (125 `unwrap()` calls, unchanged from March) remains the most fragile bridge point. Any unexpected Python error in model execution could produce a Rust panic with a cryptic backtrace instead of a Python traceback.

The `ProjectContext` API is minimal -- `find_models()` with tag and directory filtering, but no access to schemas, types, configs, or the dependency graph. A Python developer writing complex models needs richer context.

**Verdict: Wait** (unchanged)

The PyO3 migration is a genuine improvement, but the SDK distribution story (not on PyPI), the fragile bridge (`python.rs` unwraps), and the minimal Python API mean this is still not ready for Python-first workflows.

**Recommendations:**
1. Publish `smelt-sdk` to PyPI (not just `smelt-sql`)
2. Add `.pyi` type stubs for all Python APIs
3. Systematically replace `unwrap()` calls in `python.rs` with proper error handling that surfaces Python tracebacks
4. Expand `ProjectContext` with schema access, type information, and dependency graph traversal

---

## Cross-Cutting Themes

### Theme 1: The Testing Gap is Closed

The March review's universal blocker -- no data testing framework -- has been resolved. `smelt test` supports whole-model, CTE-level, property-based, and SQL body tests with comparison options (set/ordered, tolerance, type coercion). Every production-oriented perspective noted this improvement.

### Theme 2: Impressive Execution Velocity

Growing from 55K to 85K LOC in two weeks while shipping 7 of 11 recommended improvements and adding unrecommended features (schema evolution, seeds, 3 LSP features, smelt-state, smelt diff, smelt history, smelt backbuild) is extraordinary output. This velocity is both a strength (the project is converging on production-readiness quickly) and a risk (it reinforces the single-author dependency).

### Theme 3: DuckDB is Production-Ready, Spark is Evaluable

The story has shifted from "DuckDB only" to "DuckDB production, Spark beta." The Spark backend via PyO3/PySpark is functional but untested at scale and has no CI coverage. Teams on DuckDB can adopt now. Teams on Spark can evaluate. Teams on PostgreSQL, Snowflake, or BigQuery still cannot use smelt.

### Theme 4: Observability is Improving but Incomplete

println! dropped from 320 to 220 (-31%) and tracing usage grew from 8 to 32 (+300%). The trend is correct but far from complete. The smelt-state crate provides run history and interval tracking, which is the beginning of operational observability. But there are still no structured logs, no metrics export, no health checks, and no monitoring hooks.

### Theme 5: Community Infrastructure Remains the Weakest Area

Despite shipping CONTRIBUTING.md and a documentation site, there is still no Code of Conduct, no issue templates, no PR templates, no community channels, no "good first issue" labels, and no external contributors. The bus factor remains 1. For a project approaching production-readiness, this is the most critical non-technical gap.

---

## Summary Matrix

| Perspective | March Verdict | April Verdict | Top Strength | Top Concern | Priority Recommendation |
|---|---|---|---|---|---|
| dbt User | Wait | **Cautious Adopt** | Testing framework + type safety | No macro/package system | dbt migration guide |
| Director of Engineering | Pass | **Wait with Interest** | Execution velocity, feature completeness | Bus factor of 1 | Time-boxed pilot on DuckDB |
| SQLMesh User | Pass | **Pass** | LSP + testing | No virtual environments | Plan/apply workflow |
| Senior Data Architect | Wait | **Wait (stronger)** | Schema evolution, smelt-state | No lineage export | OpenLineage integration |
| Senior Analytics Engineer | Adopt (DuckDB) | **Adopt (DuckDB)** | LSP rename/references/actions | No SQL reuse mechanism | SQL templates |
| Data Engineer (Spark) | Hard Pass | **Cautious Evaluate** | Functional PyO3/PySpark bridge | Zero Spark CI tests | Spark integration tests |
| Data Analyst | Cautious Adopt | **Adopt** | pip install + docs site | unwrap() error messages | Cookbook examples |
| Senior Rust Architect | Strong Foundation | **Maturing Well** | CLI decomposition, smelt-state | 12 unsafe blocks | Audit unsafe + unwrap |
| Senior Rust Developer | Good | **Good** | Contributing guide, 1504 tests | No community infrastructure | Code of Conduct + templates |
| Senior Python Developer | Wait | **Wait** | PyO3 migration | SDK not on PyPI | Publish SDK + fix python.rs |

---

## Prioritized Recommendations

### Quick Wins (< 1 week each)

1. **Add CODE_OF_CONDUCT.md, issue templates, and PR templates** -- table stakes for community growth
2. **Publish `smelt-sdk` to PyPI** -- unblocks Python extension developers
3. **Create a dbt-to-smelt migration guide** -- highest-impact documentation for adoption
4. **Add "good first issue" labels** and create 5-10 starter issues for potential contributors
5. **Publish LSP configuration examples** for Neovim, Emacs, and Helix

### High Impact (1-4 weeks each)

1. **Audit and fix `python.rs` unwrap() calls (125)** -- the single most fragile user-facing code path; replace with error handling that surfaces Python tracebacks
2. **Add Spark integration test infrastructure** -- Docker Compose with Spark standalone; CI validates Spark backend
3. **Continue println! -> tracing migration** -- 220 println! calls remain; add structured spans for the compilation pipeline
4. **Audit 8 unsafe blocks in schema_tracking.rs** -- document safety reasoning or refactor to safe alternatives
5. **Implement a plan/apply workflow** -- `smelt diff` exists; add an approval gate and rollback mechanism

### Strategic (1-3 months each)

1. **Build PostgreSQL backend** -- the most-requested backend after DuckDB and Spark; enables cloud-native deployments
2. **Implement OpenLineage export** -- enables catalog integration (DataHub, Amundsen, Atlan)
3. **Add orchestrator integration** -- Dagster and Airflow adapters for production scheduling
4. **Build a package/dependency system** -- enable reusable model libraries
5. **Plan Salsa 0.18+ migration** -- improved APIs and performance; cost grows with codebase size
6. **Attract a second maintainer** -- the most important non-technical investment for long-term viability

---

## Appendix: Codebase Statistics

### Lines of Code by Crate

| Crate | LOC | Tests | Purpose |
|---|---|---|---|
| smelt-cli | 18,062 | 160 | CLI entry point, orchestration, graph building |
| smelt-db | 17,726 | 289 | Salsa incremental queries, type inference |
| smelt-parser | 9,483 | 263 | Rowan CST parser, lexer, AST |
| smelt-lsp | 9,469 | 129 | Language Server Protocol implementation |
| smelt-state | 6,747 | 189 | Run manifests, interval tracking, schema tracking |
| smelt-core | 4,404 | 99 | Project discovery, config, dependency graphs |
| smelt-parser-compat | 3,775 | 148 | Cross-dialect conformance testing |
| smelt-planner | 2,938 | 63 | Temporal analysis, optimization rules |
| smelt-ui | 2,416 | 5 | Axum web dashboard with embedded React |
| smelt-datagen | 2,385 | 8 | Test data generation |
| smelt-types | 2,051 | 47 | Core data type definitions |
| smelt-bench | 2,018 | 12 | Performance benchmarks |
| smelt-dialect | 1,613 | 76 | Multi-dialect SQL printer |
| smelt-backend-spark | 926 | 16 | Spark/PySpark execution backend |
| smelt-backend-duckdb | 616 | 0 | DuckDB execution backend |
| smelt-backend | 443 | 0 | Backend trait definition |
| **Total** | **85,072** | **1,504** | |

### Change from March 26 Review

| Crate | March LOC | April LOC | Delta | Notes |
|---|---|---|---|---|
| smelt-cli | 12,320 | 18,062 | +5,742 | Decomposed, testing framework, schema evolution, new commands |
| smelt-db | 9,154 | 17,726 | +8,572 | Type inference expansion, new queries |
| smelt-parser | 7,956 | 9,483 | +1,527 | Parser improvements |
| smelt-lsp | 3,270 | 9,469 | +6,199 | Rename, references, code actions |
| smelt-state | 0 | 6,747 | +6,747 | New crate |
| smelt-core | 3,723 | 4,404 | +681 | Seeds, source expansion |
| smelt-types | 1,390 | 2,051 | +661 | Type system expansion |
| smelt-dialect | 996 | 1,613 | +617 | Snapshot tests, new dialects |
| smelt-backend-spark | 267 | 926 | +659 | From stub to functional |

### Quality Metrics Comparison

| Metric | March 26 | April 9 | Change | Trend |
|---|---|---|---|---|
| Total Rust LOC | 55,093 | 85,072 | +54% | -- |
| Test functions | 919 | 1,504 | +64% | Better (outpaces LOC growth) |
| `unwrap()` calls | 935 | 1,157 | +24% | Improved ratio (grew slower than LOC) |
| `println!()` calls | 320 | 220 | -31% | Better (absolute reduction) |
| `tracing::` calls | 8 | 32 | +300% | Better (4x more structured logging) |
| `unsafe` blocks | 4 | 12 | +200% | Worse (needs audit) |
| Fuzz targets | 2 | 2 | 0% | Unchanged |
| CI workflows | 9 | 9 | 0% | Unchanged |
| Snapshot tests | 0 | 30 | -- | New |
| CLI main.rs lines | 2,387 | 414 | -83% | Major improvement |
| Clippy warnings | 0 | 0 | 0% | Enforced in CI |

### Key Dependencies

| Dependency | Version | Purpose |
|---|---|---|
| rowan | 0.15 | Lossless CST representation |
| salsa | 0.16 | Incremental computation framework |
| datafusion | 43 | SQL type coercion rules (validation only) |
| duckdb | 1.4.4 | Execution backend + test oracle |
| arrow | 58 | Data interchange format |
| parquet | 58 | Data storage format |
| tower-lsp | 0.20 | LSP protocol implementation |
| tokio | 1 | Async runtime |
| thiserror | 2.0 | Structured error types |
| anyhow | 1.0 | Error context propagation |
| proptest | 1.4 | Property-based testing |
| pyo3 | 0.28 | Python embedding (ABI3, Python 3.9+) |
| insta | 1 | Snapshot testing |
| clap | 4 | CLI argument parsing |
| serde | 1 | Serialization |
