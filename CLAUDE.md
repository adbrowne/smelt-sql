# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**smelt** - Modern data transformation framework

A next-generation data pipeline tool designed to improve upon dbt by:
- Separating logical specification (what to compute) from physical execution (how to execute)
- Enabling optimization across models
- Supporting multi-backend execution (DuckDB, Databricks, etc.)
- Using a proper language instead of Jinja templates

**Project Status**: This is still an experiment - Andrew is testing it out to see how far he can push this idea. Consider this early-stage development - no backward compatibility constraints. The codebase is evolving rapidly and breaking changes are expected. We are trying to push towards production ready from a feature perspecitve - we want to ensure we aren't backing ourselves into a corner.

## Architectural invariants

These rules constrain how the codebase evolves; the spec is the authoritative source. Read the linked spec section before changing behaviour in the named area.

- **Salsa purity (`smelt-db`)** — Analysis logic is pure functions; Salsa queries are thin wrappers that build inputs and call them. Authoritative spec: [`docs/specs/architecture.md` §"Salsa purity rule (analysis)"](docs/specs/architecture.md#salsa-purity-rule-analysis).
- **Workspace loading parity (CLI ↔ LSP)** — Eager workspace discovery lives in exactly one place (`smelt_core::workspace::load_workspace`); CLI and LSP both consume it. Authoritative spec: [`docs/specs/architecture.md` §"Workspace loading parity rule (CLI ↔ LSP)"](docs/specs/architecture.md#workspace-loading-parity-rule-cli--lsp).
- **Project isolation** — A workspace folder may contain multiple smelt projects; each is a closed resolution scope. Every workspace-scoped resolver must be project-scoped. Authoritative spec: [`docs/specs/architecture.md` §"Project isolation rule"](docs/specs/architecture.md#project-isolation-rule).
- **Run pipeline parity (CLI ↔ UI)** — The compile + execute pipeline lives in exactly one place (`smelt-runtime`); CLI and UI both consume it via `execute_project(request, reporter)`. `smelt-runtime` internals (`SqlCompiler` constructors, `PrintContext` builders, emitter factories, `compile_with_sql`) are `pub(crate)`; a consumer can only obtain a `CompiledModel` through `execute_project` or `CompilerRegistry::get(...).compile_with_sql_and_ephemerals(...)`. Authoritative spec: [`docs/specs/architecture.md` §"Run pipeline parity rule (CLI ↔ UI)"](docs/specs/architecture.md#run-pipeline-parity-rule-cli--ui). Standing CI gate: `cargo test -p smelt-runtime --test execute_parity`.
- **Diagnostic range encoding** — Diagnostics carry `rowan::TextRange` internally; conversion to `(line, column)` happens exactly once at the boundary, backed by `line_index::LineIndex`. Authoritative spec: [`docs/specs/architecture.md` §"Diagnostic range encoding rule"](docs/specs/architecture.md#diagnostic-range-encoding-rule).
- **Layered single-ownership (`smelt-logical`)** — `smelt-db` has **no** production dependency on `smelt-planner`. The logical `Plan`/`LogicalNode` model, `RuleContext`, `detect_builtin_rules`, and the pure rule-data classifiers live in `smelt-logical` (above `smelt-core`/`smelt-parser`/`smelt-types`, below both `smelt-db` and `smelt-planner`). Rule *application* stays in `smelt-planner`. Structural assertion: `cargo tree -p smelt-db -i smelt-planner` shows no production path. Authoritative spec: [`docs/specs/architecture.md` §"Layered single-ownership"](docs/specs/architecture.md#layered-single-ownership).
- **Property composition walk (`smelt-logical`)** — A composition-relevant model-property verdict (bound/reach, partition-alignment admission, the event-time monotonicity trace, grain/FD/determinism folding) is produced by the shared bottom-up walk in `analysis/walk.rs`, never by an ad hoc scan over the model's raw SQL text. A surviving non-walk scan is admissible only as a **leaf classifier** the walk invokes over one already-bounded node's own text, or as an **advisory heuristic** that never feeds admission or a derived bound — and must be classified as such in a doc comment. Authoritative spec: [`docs/specs/architecture.md` §"Property composition walk rule"](docs/specs/architecture.md#property-composition-walk-rule), [`docs/specs/model_properties.md` §Constraints "Composition happens in the walk, not in scans"](docs/specs/model_properties.md#constraints--invariants). Standing CI gate: `cargo test -p smelt-logical --test walk_coverage`.
- **Maintenance-plan purity** — the maintenance plan (per-cell technique assignment, clamps, ledger grading, propagation edges) is pure data, derived once by pure functions in `smelt-logical`; consumers (`smelt-db` diagnostics, `smelt-planner` rule application, `smelt-runtime` lowering, the graph layer) never re-derive it. The rule extends to the statements: every maintenance statement a run executes is the output of a pure emitter in `smelt-logical`'s maintenance layer — backends execute, never author (ledger DDL/DML in `smelt-state` excluded as bookkeeping). Upheld by convention for the plan-derivation half (structural assertion tracked in `docs/plans/20260707-maintenance-plan-impl.md`); the statement-level rule has a standing CI gate: `cargo test -p smelt-runtime --test statement_parity` (per-family executed-vs-emitted parity plus the structural no-authoring leg). The equivalence the derived plan actually promises (`incremental_state(S) == full_refresh(inputs ∈ S)` for every maintained model under any valid run sequence) has its own standing generative CI gate: `cargo test -p smelt-cli --test maintenance_conformance` (a deterministic-seeded sample of typed model recipes, staged and driven through the real `execute_project` pipeline against a real DuckDB backend, asserted equal to a full-refresh oracle after every run step). Authoritative spec: [`docs/specs/architecture.md` §"Constraints & Invariants"](docs/specs/architecture.md#constraints--invariants) item 12, [`docs/specs/incremental_models.md` §"Statement emission (single owner)"](docs/specs/incremental_models.md#statement-emission-single-owner), [`docs/specs/incremental_models.md` §"The equivalence invariant"](docs/specs/incremental_models.md#the-equivalence-invariant).
- **Contract-lattice point single ownership** — a declared relaxation of the equivalence invariant (the contract lattice's default point plus its declared points — `frozen_horizon`, `deferral`, and `retain_departed`) is admissible only as a complete triple single-owned in `smelt-logical`: a declaration schema, a pure oracle transform, and a probe emitter. The conformance gate consumes the oracle transform directly rather than encoding its own comparator, and runtime probes emit from the same definition — no lattice point is ever defined ad hoc by a caller. Authoritative spec: [`docs/specs/incremental_models.md` §"The contract lattice"](docs/specs/incremental_models.md#the-contract-lattice).
- **Fail-loud discipline** — Every path that can encounter unrecognisable user input must emit a diagnostic rather than silently falling back to a default or `Unknown` value. Four CI gates enforce this; never lower them without a reviewer sign-off note in the commit:
  - **`unwrap`/`expect` ratchet** (`cargo test -p smelt-core --test hardening_budget::gate_detects_regression`) — per-crate production `unwrap` and `.expect("` counts must not exceed `.claude/hardening-baseline.txt`. New production `unwrap`/`expect` must be classified as infallible or converted to `Result`. "Production" excludes test-support crates, *derived* rather than listed: a crate some crate dev-depends on, that no crate depends on normally, and that produces no binary. The baseline is two-sided in both directions — a count that falls, and an entry for a crate no longer counted, are each an error telling you to re-run `--update`.
  - **`println!` gate** (`cargo test -p smelt-core --test hardening_budget::no_println_in_libraries`) — zero production `println!` in all library crates. Use `tracing::{debug,info,warn}` instead. Legitimate user-facing stdout in `smelt-cli`/`smelt-ui` is excluded.
  - **`error`-Unknown guard** (`cargo test -p smelt-types --test unknown_census::every_unknown_site_is_classified`) — every `DataType::Unknown` construction site in production code must be classified in `.claude/unknown-census.toml` as `legitimate` or `error`. An unclassified new site fails the test.
  - **`MetadataError` exhaustiveness gate** (compiler-enforced) — `map_metadata_error_to_diagnostic` in `smelt-db/src/lib.rs` is an exhaustive `match` over all `MetadataError` variants; adding a new variant to `smelt-core/src/metadata.rs` without listing it here is a compile error. `None` is only permitted for variants handled by a dedicated arm elsewhere in `check_file_diagnostics`.
  Authoritative spec: [`docs/specs/architecture.md` §"Fail-loud discipline"](docs/specs/architecture.md#fail-loud-discipline). Diagnostic-code catalogue: [`docs/specs/diagnostics.md`](docs/specs/diagnostics.md).
- **SQL dialect conformance gates** — the parser's and type-inference's dialect claims are verified differentially against a real DuckDB, in both directions, never lowered without a reviewer sign-off note:
  - **DuckDB differential + fidelity** (`cargo test -p smelt-parser-compat --test duckdb_differential`) — every statement DuckDB accepts must parse cleanly or match a `gaps.rs` entry (accept direction), and every clean parse must print back to SQL DuckDB still executes (fidelity direction). The registered-gap count ratchets down only (`.claude/parser-gaps-baseline.txt`).
  - **External corpus ledger** (`cargo test -p smelt-parser-compat --test external_corpus`) — a vendored DuckDB/PostgreSQL SELECT corpus runs the same parse-or-registered check against a shrink-only failure ledger.
  - **Type-oracle strictness** (`cargo test -p smelt-db --test type_property_tests`) — inferred types are compared to the engine's schema exactly (integer width and Decimal precision/scale included). The only blanket compatibility rule is the named `text_varchar_compat` string-family leniency; every other tolerated difference is an explicit `divergences.rs` entry, and every `Unknown` inference must match a `known_unknowns.rs` entry rather than being silently skipped. Cast-wrapped decimal correctness is separately asserted with zero divergence by `cargo test -p smelt-db --test proptests` (`type_conformance_tests`).
  Authoritative spec: [`docs/specs/architecture.md` §"Constraints & Invariants"](docs/specs/architecture.md#constraints--invariants) item 13.
- **Function-registry single ownership** — a built-in SQL function's name, classification (aggregate/window/scalar), registry-driven type, **and per-dialect, per-position emission** all derive from one table (`BuiltinRegistry` in `crates/smelt-types/src/signatures.rs`), never lowered without a reviewer sign-off note. Emission verdicts are keyed on `(DialectId, Position)` — `Any`/`Scalar`/`Aggregate`/`WholePartitionWindow`/`Window` — because a backend's support for a built-in routinely differs between positions; there is no position-blind lookup:
  - **Consistency gate** (`cargo test -p smelt-db --test integration registry_consistency::every_recognized_function_is_registry_backed`) — every name `SqlFunction` recognises resolves in the registry with a matching classification, and every non-operator registry entry is a recognised function; a name in one list but not the other fails with the missing side named.
  - **Migration ratchet** (`cargo test -p smelt-db --test integration registry_consistency::legacy_match_ratchet`) — the count of functions still typed by the hand-written `match` in `type_inference/function_call.rs` (not registry-first via `try_registry_inference`) ratchets down only (`.claude/registry-migration-baseline.txt`).
  - **Emission-ownership gate** (`cargo test -p smelt-dialect --test emission_ownership`) — `printer.rs` holds no name-matched function spelling, no branch on a concrete `SqlDialect` variant, and no derivation of a call's *position* (that is the compile path's question to the registry, not the printer's to infer). A per-dialect spelling is `Signature::emission` data; a capability-shaped difference is a `BackendCapabilities` flag. Every `RewriteId` **and** `RestructureId` variant (parsed out of `signatures.rs`, not restated) must be dispatched.
  - **Statement-level lowering** — where a backend offers a built-in only in the opposite position from the one the author wrote, the lowering restructures the statement around a synthesised CTE (`Emission::Restructure`), planned as a pure function of the **source** CST before printing and never recovered from printed SQL. A running-frame window over a built-in with no analytic form on the target has no correct CTE form and is refused with `UnsupportedOnBackend`. The synthesised join is null-safe (`IS NOT DISTINCT FROM`, `<=>` on Spark) so a NULL partition key cannot silently drop rows. Authoritative spec: [`docs/specs/multi_backend.md` §"Statement-level lowering"](docs/specs/multi_backend.md#statement-level-lowering).
  - **Cross-engine emission audit** (`cargo test -p smelt-db --test dialect_audit`) — probes derived from the registry, not authored against it, run against live engines in two legs: schema (does the printed SQL run?) and value (does it compute the same thing? — the leg that catches `^`, which is bitwise XOR on Spark and GoogleSQL but power in smelt). Gated four ways: coverage totality (an entry with no probe is named, never dropped), the two-sided `ledger.rs` (an unregistered mismatch fails; so does a row the engine now accepts, or one naming a pair that is never probed), the `Gap` ratchet in `.claude/dialect-gaps-baseline.txt`, and the doc-sync gate on the generated `docs/reference/dialect-coverage.md`. DuckDB's legs run per-PR in-process; Spark's run nightly or on a `run-docker-tests` PR; BigQuery stays a manual sweep (`scripts/bigquery-dialect-audit.sh`) because its value leg executes rather than dry-runs.
  - **Compile-path refusal** (`cargo test -p smelt-runtime --test dialect_seam`) — a construct the registry declares `Emission::Unsupported` on the target's dialect fails at compile time with `UnsupportedOnBackend`, and no compile entry point may reach the printer without that check. The same suite guards the printer → cast-wrap → projection seam, where the `MEDIAN` re-parse bug lived.

  Authoritative spec: [`docs/specs/architecture.md` §"Constraints & Invariants"](docs/specs/architecture.md#constraints--invariants) item 14, [`docs/specs/multi_backend.md` §"Operator lowering"](docs/specs/multi_backend.md#operator-lowering) and §"Cross-engine emission audit". Published coverage table: [`docs/reference/dialect-coverage.md`](docs/reference/dialect-coverage.md).
- **Source-derived projection** — a model's projection (its output column names and their inferred types) is derived once from the model's **source** select list, before dialect lowering; no compile entry point may recover it by re-parsing the dialect-printed SQL, since a backend's own lowering (`MEDIAN`, `%`, `QUALIFY`, …) does not parse back as smelt SQL. Standing gate: `cargo test -p smelt-runtime --test projection_dialect_invariance` — compiles one model exercising every construct the printer lowers for DuckDB, Spark and BigQuery and asserts `output_columns` and the cast-wrap column names are byte-identical across all three; it needs no live warehouse and runs per-PR. Authoritative spec: [`docs/specs/multi_backend.md` §"Output-schema type conformance"](docs/specs/multi_backend.md#output-schema-type-conformance).

## Key Documentation

- **docs/specs/**: Per-feature normative specs — the canonical answer to "how does this feature work?"
  - Naming convention: `<feature>.md` (e.g., `incremental_models.md`, `architecture.md`)
  - See `docs/specs/SPEC_TEMPLATE.md` for the file format
  - Spec-first rule: edit the spec before writing the plan that changes the feature

- **README.md**: Project overview, quick example, and current status snapshot
  - For normative behavior, see `docs/specs/`

- **docs/DESIGN.md**: Legacy design document — being incrementally extracted into `docs/specs/`
  - Authoritative for areas not yet specified in `docs/specs/`
  - As specs are extracted, this thins out

- **docs/ROADMAP.md**: Implementation status and next steps
  - Track completed phases with completion dates
  - Document deferred work with rationale
  - Propose concrete next-step options
  - **Update after completing phases or making architectural decisions**

- **docs/**: Additional design documentation
  - `lsp_architecture.md`: LSP implementation details
  - `lsp_quickstart.md`: Getting started with the LSP
  - `planner_rule_api_design.md`: Future planner API design
  - `type_semantics.md`: Type inference rules

- **docs/research/**: Research into an idea. Often the predecessor to a spec or plan.
  - Naming convention: `YYYYMMDD-short-name.md` (e.g., `20260321-planner-api.md`)

- **docs/plans/**: Implementation plans committed to the repo
  - Naming convention: `YYYYMMDD-short-name.md` (e.g., `20260321-planner-api.md`)
  - Each plan cites the spec it implements and the spec diff
  - Created before non-trivial implementation work, via `/smelt:plan`

## Commands

### Build and Test (System DuckDB - Recommended)

Using system DuckDB avoids recompiling DuckDB from C++ source, making builds much faster.

**Setup:** Install the DuckDB shared library (v1.5.4) and set `DUCKDB_LIB_DIR`:
```bash
# Download and install (one-time setup)
curl -sL https://github.com/duckdb/duckdb/releases/download/v1.5.4/libduckdb-linux-amd64.zip -o /tmp/libduckdb.zip
cd /tmp && unzip -o libduckdb.zip libduckdb.so
sudo cp libduckdb.so /usr/local/lib/ && sudo ldconfig
# Or for user-local install:
mkdir -p ~/.local/lib/duckdb && cp libduckdb.so ~/.local/lib/duckdb/

# Set env var (add to ~/.bashrc or ~/.zshrc)
export DUCKDB_LIB_DIR=/usr/local/lib          # system install
# or: export DUCKDB_LIB_DIR=~/.local/lib/duckdb  # user-local install
```

**Commands (system DuckDB is now the default):**
```bash
# Build the entire workspace
cargo build

# Format code (required before committing)
cargo fmt --all

# Check formatting without modifying files
cargo fmt --all -- --check

# Run clippy (linter) - must pass with no warnings.
# Prefer the shared gate: it lints BOTH feature sets CI lints (default, and
# --no-default-features + duckdb backends), and is the same script CI runs, so
# a local pass cannot diverge from CI.
bash .claude/scripts/clippy-gate.sh

# Run tests
cargo test

# Verify example workspaces have no LSP diagnostics
cargo test -p smelt-cli --test example_diagnostics

# Standard pre-commit gate, bundled into ONE command (fmt + clippy + tests +
# example_diagnostics, failures-only output). Prefer this over running the
# four commands separately — it keeps agent transcripts small.
bash .claude/scripts/verify-phase.sh          # add --fast to skip the full cargo test

# Run the LSP server
cargo run -p smelt-lsp

# Test with sample workspace
# (Configure your editor to use the LSP server, then open examples/test_workspace/)
```

### Build and Test (Bundled DuckDB - No System Dependencies)

If you don't have the system DuckDB library, bundled mode compiles DuckDB from source (slower first build).
You must explicitly opt in with feature flags:
```bash
cargo build --features smelt-cli/bundled-duckdb,smelt-ui/bundled-duckdb
cargo test --features smelt-cli/bundled-duckdb,smelt-ui/bundled-duckdb,smelt-db/bundled-duckdb
cargo clippy --all-targets --features smelt-cli/bundled-duckdb,smelt-ui/bundled-duckdb
```

### Spark parity tests (gated — needs Delta-enabled server)

These tests require a Delta-enabled Spark Connect server. Trigger them locally with:

```bash
# 1. Start Delta-enabled Spark Connect server (downloads Delta jars on first run)
bash scripts/spark-up.sh

# 2. Export the Spark env vars (SPARK_CONNECT_URL, PYTHONPATH, PYSPARK_PYTHON, …)
source scripts/spark-env.sh

# 3. Run the full Spark parity suite (dual-target + backend-spark integration tests)
cargo test -p smelt-backend-spark --quiet 2>&1 | tail -40
cargo test -p smelt-cli --features smelt-cli/spark --quiet 2>&1 | tail -40

# 4. Tear down the server
bash scripts/spark-down.sh
```

In CI the `spark-parity` job in `.github/workflows/compat.yml` runs the same sequence. It is
gated — triggered only on `schedule` or when the `run-docker-tests` label is added to a PR.

### VSCode Extension
```bash
# Install and build the extension
cd editors/vscode
npm install
npm run compile

# Test in development mode
# Open editors/vscode in VSCode and press F5 to launch Extension Host

# Package as VSIX (requires Node 18+)
npm run package

# Watch mode (auto-recompile on changes)
npm run watch
```

## Token efficiency

These rules apply to every subagent and every Bash invocation. The autonomous
loop dispatches dozens of subagents per phase, so one large tool result repeated
across phases is a real cost.

- **Search with `rg` (ripgrep), not `grep -r` or `find`.** `rg` honors
  `.gitignore` by default; `grep -r`/`find` traverse `docs-site/site/`,
  `target/`, `node_modules/`, and other build artifacts. A single `grep -r`
  over the workspace has previously returned 2+ MB of minified JS source maps
  from `docs-site/site/assets/javascripts/*.min.js.map` — ~500K tokens dropped
  into one tool result.
- **Never search into build artifacts.** Specifically:
  `docs-site/site/`, `target/`, `examples/*/target/`, `examples/*/.smelt/`,
  `ui/dist/`, `ui/node_modules/`, `__pycache__/`. If you must use `grep`,
  pass `--exclude-dir=site --exclude-dir=target --exclude-dir=node_modules`.
- **`cargo test` output:** for verification gates use
  `cargo test --quiet 2>&1 | tail -40`. The full pass listing of the workspace
  is ~3,000 lines per run — only worth feeding back when investigating an
  actual failure. Failures still surface; their context is in the tail.
- **`cargo build` / `cargo check`:** silence routine warnings the same way:
  `2>&1 | tail -50`. The first failure line is what matters.
- **`git diff` to a reviewer:** prefer per-crate or per-file diffs over
  whole-repo diffs when the change is bounded to one area.

A usage log is written to `.claude/usage-log.jsonl` by the PostToolUse and
SessionEnd hooks (see `.claude/settings.json`) and by `autonomy-loop.sh` for
headless iterations. Summarize it with
`bash .claude/scripts/usage-summary.sh` to see top tool-result outliers and
per-iteration cost after an autonomous run.

## Autonomy loop

The autonomy loop drives plan work headlessly: each iteration spawns a fresh
`claude --print`, executes the next `pending` phase of the active sub-plan
(named via `.claude/active-plan`), and emits a sentinel
(`<<PHASE_COMPLETE>>` / `<<PHASE_BLOCKED>>` / `<<SUBPLAN_ADVANCED>>` /
`<<MASTER_EXHAUSTED>>` / `<<ALL_DONE>>`) that the wrapper
(`.claude/scripts/autonomy-loop.sh`) dispatches on. Blocked phases are
recorded and skipped, never stop-the-line.

**Operator guide — how to launch/stop/tune it, memory isolation, logs,
and the "ask Claude to start it" prompt — lives in
[`docs/autonomy_loop.md`](docs/autonomy_loop.md).** Never launch the loop
from inside a Claude session with a bare backgrounded Bash call; use a
detached tmux session or systemd unit per that doc.

## Outcome loop

A parallel headless loop for **outcome-driven** work: the committed artifact
is an outcome (goal + checkable success criteria + one-line phase intents in
`docs/outcomes/<name>/outcome.md`), and each phase's detailed plan is written
just-in-time by an Opus plan step that reads the previous phase's summary and
may reshape the remaining phases — work serving the success criteria is never
deferred out. A Sonnet implement step then executes the plan. Outcomes queue
in `.claude/outcome-backlog` (priority order; the loop advances through it).
Scaffold with `/smelt:outcome`; driver `.claude/scripts/outcome-loop.sh`;
operator guide in
[`docs/outcome_loop.md`](docs/outcome_loop.md). Never run both loops in the
same checkout simultaneously.

## Architecture

### High-Level Design

smelt is a **compiler and orchestrator**, not a query engine:
```
User DSL → Parser → Logical CST → Planner → Physical CST → SQL for Target Engines
```

- **Logical CST**: Represents WHAT to compute (correctness specification)
- **Physical CST**: Represents HOW to execute (engine selection, materialization decisions)
- **Planner**: Transforms logical CST into optimized physical CST while preserving correctness. Supports custom rules.

### Parser Architecture

The parser is separated into reusable layers:
```
smelt-parser (pure parser)  →  smelt-db (Salsa queries)  →  smelt-lsp (LSP server)
                          ↘  smelt-planner
                          ↘  smelt-cli
```

- **smelt-parser**: Standalone Rowan-based parser (no Salsa dependency)
  - Pure function: text → CST with error recovery
  - Reusable in any context (LSP, planner, CLI)
  - Fast one-shot parsing for non-incremental use cases

- **smelt-db**: Salsa wrapper around smelt-parser
  - Incremental compilation for LSP responsiveness
  - Caches parse results and derived queries
  - Automatic invalidation when files change

This separation allows the LSP to get incremental parsing via Salsa, while the planner and CLI can use fast one-shot parsing directly from smelt-parser.

### Key Dependencies

- **Salsa**: Incremental computation framework (enables fast recompilation and LSP)
- **Rowan**: Lossless CST library (error-recovery parser foundation)
- **tower-lsp**: Language Server Protocol implementation
- **DuckDB**: One of the supported backends and extensively used for testing.

### Examples

All examples live under `examples/`:

- **`examples/timeseries/`**: User/event analytics pipeline (12 SQL models, incremental materialization)
- **`examples/retail_analytics/`**: TPC-DS-based retail pipeline (25 models: staging/intermediate/marts)
- **`examples/broken/`**: Intentionally broken models for testing error handling
- **`examples/test_workspace/`**: Minimal workspace for VSCode/LSP integration testing
- **`examples/huge/`**: Auto-generated 2000-model workspace for stress testing

### User documentation

User documentation lives under docs-site/. For any user facing feature change consider whether documentation addition/update is required.

### Crate Structure

- `smelt-parser`: Rowan-based error-recovery parser (standalone, reusable)
  - Lexer: Tokenizes SQL + smelt extensions (`smelt.ref()`, `smelt.metric()`, `=>` operator)
  - Parser: Recursive descent parser with error recovery at sync points
  - AST: Typed wrappers over Rowan CST for convenient traversal
  - Parses SQL structure: SELECT, FROM, WHERE, GROUP BY, expressions, functions
  - Named parameters: Handles `param => value` syntax in function calls
  - Position tracking: Accurate line/column information for diagnostics and goto-definition
- `smelt-db`: Salsa database with incremental queries (wraps smelt-parser for incremental compilation)
  - Input queries: `file_text()`, `all_files()`
  - Syntax queries: `parse_file()`, `parse_model()`, `model_refs()` (with positions)
  - Semantic queries: `resolve_ref()`, `file_diagnostics()` (with accurate positions)
- `smelt-lsp`: Language Server Protocol implementation
  - Diagnostics for undefined refs and parse errors (with accurate positions)
  - Go-to-definition for `smelt.ref()` using CST position tracking
  - Extracts named parameters from ref calls for future validation
  - Full Salsa integration for incremental updates
- `editors/vscode`: VSCode extension
  - Language client that connects to smelt-lsp
  - Syntax highlighting for SQL + templates
  - Auto-activation when models/ directory detected
  - See editors/vscode/README.md for installation

## Key Differentiators from dbt

1. **Logical/Physical Separation**: Users specify logic in models, possibly with some attributes that specify how planner rules should work, planner rules specify how it actually runs (for example turning a full table query into an incrimental model)
2. **Engineer controls planning**: Planner is not a black box - the API will allow data engineers to refactor specific logical plans to optimize - the framework should make it easy to do these and know that correctness is preserved - or where not - what has been lost.
3. **Cross-Model Optimization**: Can fuse or split queries across model boundaries
4. **Multi-Backend**: Automatically distribute work across engines (e.g., DuckDB for small data, Databricks for large)
5. **Proper Language**: No Jinja templates, proper compilation with type checking
6. **First-Class Editor Support**: LSP with incremental compilation via Salsa
   - Real-time diagnostics and completions
   - Error-recovery parser handles partial/invalid code
   - Incremental recompilation for fast feedback

## Type Property Tests

Property-based tests verify smelt's type inference against DuckDB:

**Type correctness oracle** (`crates/smelt-db/tests/type_property_tests.rs`):
```bash
# Run all property tests (256 cases + smoke tests)
cargo test -p smelt-db --test type_property_tests

# Run only the property test
cargo test -p smelt-db --test type_property_tests prop_type_inference

# Deeper coverage (local only)
PROPTEST_CASES=1000 cargo test -p smelt-db --test type_property_tests prop_type_inference
```

**Nullability soundness oracle** (`crates/smelt-db/tests/nullability_property_tests.rs`):
```bash
# Run all nullability tests (256 cases + smoke tests)
cargo test -p smelt-db --test nullability_property_tests

# Run only the property test
cargo test -p smelt-db --test nullability_property_tests prop_nullability_sound

# Deeper coverage (local only)
PROPTEST_CASES=1000 cargo test -p smelt-db --test nullability_property_tests prop_nullability_sound
```

**Structure:**
- `tests/prop_helpers/generators.rs` — Type-aware SQL expression generators
- `tests/prop_helpers/arrow_mapping.rs` — Arrow → smelt DataType mapping
- `tests/prop_helpers/duckdb_oracle.rs` — DuckDB execution oracle (trait-based for future PG/Spark); also provides value-based `count_nulls_per_column` and `execute_ddl` for nullability tests
- `tests/prop_helpers/null_data.rs` — NULL-bearing data generation for nullability soundness tests
- `tests/prop_helpers/type_comparison.rs` — Exact/Compatible/Mismatch comparison
- `tests/prop_helpers/divergences.rs` — Known type divergence registry

**When a proptest failure occurs:**
1. Check if it's a known divergence → add to `divergences.rs`
2. Check if types are compatible (Text/Varchar, Decimal precision) → add to `type_comparison.rs`
3. Otherwise fix the inference in `smelt-db/src/type_inference.rs`

## Development Workflow

**Git Workflow:**
- Work on local branches for non-trivial changes, then push a branch and create GitHub PR.
- For small/trivial changes, committing directly to `main` is fine
  - No PRs needed — just push to `main` after tests pass
- Normal git operations including push are available

## When writing code

- Always use red-green testing. Stop writing code when the test passes. Update the test or add a new one if you don't think the task is complete.
- When you discover a property based test failure, add an explicit test to capture that failure before fixing the bug.
- **Spec-first for feature behavior changes.** If you're changing the user-visible surface or semantics of a feature, edit `docs/specs/<feature>.md` first. Plans for that feature must cite the spec and ideally the spec diff. If no spec exists yet for the feature, create one as part of the work — extract from `DESIGN.md` and existing plans. See `docs/specs/SPEC_TEMPLATE.md`.
- **Plans update code and user docs together.** Unless the plan header says `Docs: code-only`, every plan must include phases that update `docs/specs/<feature>.md` Surface section and the corresponding `docs-site/` pages alongside the implementation.

### For Parser/LSP Features

1. Review the spec in README.md for requirements
2. Implement parser changes (lexer → syntax → parser → AST)
3. Update smelt-db queries if needed (usually automatic via AST)
4. Update LSP features if needed (diagnostics, goto-definition, etc.)
5. Test with examples/test_workspace models
6. **Run `cargo fmt --all` to format code**
7. **Run `bash .claude/scripts/verify-phase.sh`** — the bundled gate (fmt-check + clippy zero-warnings over both CI feature sets + `cargo test` + example_diagnostics) in one command with failures-only output
8. **Run `cargo test -p smelt-lsp --test example_workspaces`** to verify examples have no diagnostics via the real LSP backend (catches asymmetric-discovery bugs the Salsa-direct test misses)
9. Update docs/ROADMAP.md with completion status and date
10. **Commit** with descriptive message (includes ROADMAP.md update)

### Before Ending a Conversation

Before wrapping up, write any unfinished work and open decisions to `docs/TODO.md` and update this `CLAUDE.md` with anything a fresh context should know.

## Maintaining docs/ROADMAP.md

**When to update:**
- After completing a phase (mark as ✅ with completion date)
- When deferring work (mark as ⏸️ with rationale)
- When proposing new next steps (add as Option)
- When making architectural decisions (document reasoning)

**Format:**
- Use ✅ for completed phases
- Use ⏸️ for deferred work
- Use 🔄 for in-progress work
- Use 🔮 for future/speculative work
- Always include completion dates for finished work (e.g., "December 26, 2024")
- Always explain why work is deferred

**Note:** Use dates instead of commit hashes to avoid requiring a follow-up commit just to document the hash.

## Specs

Per-feature specs live under `docs/specs/` as markdown files.

**Naming convention:** `docs/specs/<feature>.md` (e.g., `incremental_models.md`, `architecture.md`).

A spec is the **canonical answer** to "how does this feature work?". It is normative — the implementation and the user docs must match it. See `docs/specs/SPEC_TEMPLATE.md` for the file format.

- Edit the spec **before** writing the plan that changes the feature.
- The spec diff is the change description for `/smelt:plan`.
- `/smelt:validate <feature>` produces a drift report against the spec.

### Timeless-oracle rule

Specs (`docs/specs/`) and user docs (`docs-site/docs/`) describe the feature as if it has always existed. They never reference plan phases, milestones, or implementation history. A reader six months after the plan ships should not be able to tell which plan introduced a given paragraph.

**Forbidden in spec body and user docs:**
- Section headings tagged with a plan phase: `### Phase A — List<T>`, `#### Phase C goto-def`.
- Inline labels referring to a phase: `Meta list (Phase A)`, "Phase B adds…", "ships in Phase E1".
- Status callouts written in plan vocabulary: `*[deferred to Phase E1]*`, "Phase 0 scaffold".

**Where it does belong:**
- **Plans (`docs/plans/`)** — phases live here; this is the only place phase vocabulary is allowed.
- **Spec → Known Divergences / Open Questions** — describe the gap in terms of *behavior* ("`column_origin` is not yet emitted by producers") and link the tracking plan. Phase numbers are tolerated here only when paired with a plan link.
- **Spec → References → Plans (history)** — link plan files; do not describe their internal phase structure.

**Examples — bad → good:**

| Bad (in spec/user-doc body) | Good |
|---|---|
| `### Phase A — List<T>, list literals, spread` | `### Lists and spread` |
| "Phase B adds iteration, transformation, and compile-time configuration:" | "The meta-language provides iteration, transformation, and compile-time configuration via:" |
| "Inline-schema sugar … is a Phase E1 decision." | Move to **Open Questions**: "Whether `load_yaml(path, { name: Text })` is first-class surface or sugar for a named declaration is undecided. Tracked in `docs/plans/20260509-meta-language-overall.md`." |
| "ships in Phase 51 of `docs/plans/20260422-smelt-functions.md`" inside §Surface | Move the *status* line to **Known Divergences**; the §Surface entry just describes the validation. |

`/smelt:validate` flags `Phase [A-Z0-9]` matches in spec body sections and user docs as drift.

## Plans

Plans are committed to the repo under `docs/plans/` as markdown files.

**Naming convention:** `YYYYMMDD-short-name.md` (e.g., `20260321-planner-api.md`)

- Plans cite the spec they implement (`Spec:` and `Spec diff:` header fields). Plans do not restate the spec.
- Plans include explicit `pending`/`done` Progress tracking, per-phase TDD tests, implementer + reviewer review checklists, and per-phase commit messages. The mandatory structure is encoded in `/smelt:plan`.
- **Always commit the plan file to `docs/plans/`** as part of the implementation work — do not leave it only in `.claude/plans/`.

## Workflow & Slash Commands

The workflow is spec-driven and uses Frequent Intentional Compaction. Adapted from [ACE-FCA](https://github.com/humanlayer/advanced-context-engineering-for-coding-agents) (Dex Horthy / HumanLayer) with attribution.

**Spec-driven commands** (in `.claude/commands/smelt/`):
- `/smelt:spec` — Draft or update a feature spec (outputs to `docs/specs/`)
- `/smelt:plan` — Generate a phased implementation plan from a spec diff (outputs to `docs/plans/`)
- `/smelt:implement` — Execute a plan phase-by-phase using implementer + reviewer subagents, spec as oracle
- `/smelt:validate` — Verify implementation and user docs match the spec; produce a drift report

The standard flow: `/smelt:spec → /smelt:plan → /smelt:implement → /smelt:validate`. Each phase of `/smelt:implement` runs an implementer subagent (red-green TDD on the listed tests, real-fixture coverage in `examples/`) followed by a reviewer subagent that flags only material findings, then commits and pushes atomically.

**Leverage hierarchy.** A bad line of a spec leads to thousands of bad lines of code; a bad line of a plan leads to hundreds. Spend review effort there, not on generated code:

```
spec → plan → code
1000x   100x    1x   (relative leverage of human review)
```

**Be willing to throw away.** If a spec, research doc, or plan went in the wrong direction, discard it and re-steer with better framing. The cost of restarting a phase is low compared to building on a bad foundation.

**Compaction guidance** — when compacting (manually or automatically), preserve:
- The active plan path and which phases are `done`
- The spec referenced by the plan
- List of files modified in this session
- Current build/test status
- Decisions made and their rationale

Full source documents (each artifact is committed): `docs/specs/` (specs), `docs/plans/` (plans, naming `YYYYMMDD-name.md`), `docs/research/` (`YYYY-MM-DD-topic.md`), `docs/handoffs/` (`YYYY-MM-DD-name.md`).

## Agent skills

### Issue tracker

Issues and specs live as GitHub issues on `adbrowne/smelt-sql`, managed via the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Domain docs

Single-context layout: `CONTEXT.md` + `docs/adr/` at the repo root (created lazily as concepts/decisions are resolved). See `docs/agents/domain.md`.

## License

MIT License - Copyright (c) 2025 Andrew Browne
