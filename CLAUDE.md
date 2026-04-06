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

## Key Documentation

- **README.md**: Full language specification and design decisions
  - Two-layer DSL architecture (Metrics DSL + SQL models)
  - Type system design
  - Extension syntax (`smelt.ref()`, `smelt.metric()` with `=>` parameters)
  - Computation requirements (stateless, windowed, sessionized, etc.)
  - Backend capabilities and rewrite rules
  - Incrementalization and optimization strategy

- **docs/ROADMAP.md**: Implementation status and next steps
  - Track completed phases with completion dates
  - Document deferred work with rationale
  - Propose concrete next-step options
  - **Update after completing phases or making architectural decisions**

- **docs/**: Architecture and design documentation
  - `architecture_overview.md`: System design and component interactions
  - `lsp_architecture.md`: LSP implementation details
  - `lsp_quickstart.md`: Getting started with the LSP
  - `planner_rule_api_design.md`: Future planner API design

- **docs/research/**: Research into an idea. Often the predecessor to a plan.
  - Naming convention: `YYYYMMDD-short-name.md` (e.g., `20260321-planner-api.md`)

- **docs/plans/**: Implementation plans committed to the repo
  - Naming convention: `YYYYMMDD-short-name.md` (e.g., `20260321-planner-api.md`)
  - Created before non-trivial implementation work

## Commands

### Build and Test (System DuckDB - Recommended)

Using system DuckDB avoids recompiling DuckDB from C++ source, making builds much faster.

**Setup:** Install the DuckDB shared library (v1.5.0) and set `DUCKDB_LIB_DIR`:
```bash
# Download and install (one-time setup)
curl -sL https://github.com/duckdb/duckdb/releases/download/v1.5.0/libduckdb-linux-amd64.zip -o /tmp/libduckdb.zip
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

# Run clippy (linter) - must pass with no warnings
cargo clippy --all-targets

# Run tests
cargo test

# Verify example workspaces have no LSP diagnostics
cargo test -p smelt-cli --test example_diagnostics

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

### Pure Function Rule (smelt-db)

**All analysis logic in smelt-db must be implemented as pure functions. Salsa queries must be thin wrappers that call these functions.**

This is an architectural invariant. The core type inference, schema extraction, and diagnostic checks are deliberately written as pure functions that take AST nodes and plain data structures — not Salsa database references. Salsa queries exist only to provide incrementality (caching, dependency tracking, change detection).

**Why this matters:** We plan to extract a `smelt-check` crate for batch compilation (planner, CLI) that doesn't need Salsa. Keeping logic pure makes that extraction a mechanical refactoring rather than a rewrite.

**The rule in practice:**
- **DO**: Write analysis as `fn check_something(ast: &Expr, ctx: &TypeContext) -> Result`
- **DO**: Have Salsa queries build the inputs, call the pure function, and return the result
- **DON'T**: Use `db.some_query()` calls inside analysis logic — pass the data in as parameters instead
- **DON'T**: Make `TypeContext`, `ModelSchema`, or diagnostic functions depend on Salsa traits

**Current examples of this pattern:**
- `type_inference.rs` — 1800 lines of pure functions, zero Salsa imports
- `schema.rs` — pure data structures
- `check_expression_types()` in `lib.rs` — pure diagnostic check

**Current exceptions (acceptable for now):**
- `file_diagnostics()` orchestrates multiple Salsa queries to gather inputs before running checks
- `type_context()` calls Salsa to resolve upstream model schemas

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

Property-based tests in `crates/smelt-db/tests/type_property_tests.rs` verify smelt's type inference against DuckDB:

```bash
# Run all property tests (256 cases + smoke tests)
cargo test -p smelt-db --test type_property_tests

# Run only the property test
cargo test -p smelt-db --test type_property_tests prop_type_inference

# Deeper coverage (local only)
PROPTEST_CASES=1000 cargo test -p smelt-db --test type_property_tests prop_type_inference
```

**Structure:**
- `tests/prop_helpers/generators.rs` — Type-aware SQL expression generators
- `tests/prop_helpers/arrow_mapping.rs` — Arrow → smelt DataType mapping
- `tests/prop_helpers/duckdb_oracle.rs` — DuckDB execution oracle (trait-based for future PG/Spark)
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

### For Parser/LSP Features

1. Review the spec in README.md for requirements
2. Implement parser changes (lexer → syntax → parser → AST)
3. Update smelt-db queries if needed (usually automatic via AST)
4. Update LSP features if needed (diagnostics, goto-definition, etc.)
5. Test with examples/test_workspace models
6. **Run `cargo fmt --all` to format code**
7. **Run `cargo clippy --all-targets` and fix all warnings**
8. Run `cargo build` and `cargo test` to ensure everything compiles and passes
9. **Run `cargo test -p smelt-cli --test example_diagnostics`** to verify examples have no LSP diagnostics
10. Update docs/ROADMAP.md with completion status and date
11. **Commit** with descriptive message (includes ROADMAP.md update)

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

## Plans

Plans are committed to the repo under `docs/plans/` as markdown files.

**Naming convention:** `YYYYMMDD-short-name.md` (e.g., `20260321-planner-api.md`)

- Create a plan before starting non-trivial implementation work
- Plans should be committed alongside or before the implementation they describe
- Use markdown format
- **Always commit the plan file to `docs/plans/`** as part of the implementation work — do not leave it only in `.claude/plans/`

## ACE-FCA Workflow

This project uses the [ACE-FCA workflow](docs/ace-fca-guide.md) (Advanced Context Engineering with Frequent Intentional Compaction) for non-trivial development tasks.

**Slash commands** (in `.claude/commands/`):
- `/research` — Explore codebase to understand a topic (outputs to `docs/research/`)
- `/plan-rpi` — Create an implementation plan from research (outputs to `docs/plans/`)
- `/iterate-plan` — Refine an existing plan based on feedback
- `/implement` — Execute a plan phase by phase with verification gates
- `/validate` — Verify implementation matches the plan specification
- `/handoff` — Compact session context for continuity (outputs to `docs/handoffs/`)

**Compaction guidance** — When compacting (manually or automatically), preserve:
- The active plan path and which phases are complete
- List of files modified in this session
- Current build/test status
- Decisions made and their rationale

See `docs/ace-fca-guide.md` for the full tutorial.

## License

MIT License - Copyright (c) 2025 Andrew Browne
