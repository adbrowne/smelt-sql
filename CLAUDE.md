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

### Workspace Loading Parity Rule (CLI ↔ LSP)

**Eager init-time workspace discovery lives in exactly one place: `smelt_core::workspace::load_workspace`. Both the CLI's `init_db` and the LSP's `Backend::initialize` consume it.**

This covers: SQL models under `config.paths`, function definitions under the hardcoded `functions/` directory, multi-model frontmatter expansion, and YAML/JSON/TOML loader files for `smelt.config.load_yaml`. The Salsa-ingest sequence (`set_project_input` → `set_source_file` → `register_loader_files_from_disk`) is centralised in `smelt_db::workspace_ingest::ingest_loaded_workspace`. *Lazy* discovery (seeds via `project_seeds`, per-entity sources via `project_sources`) lives inside Salsa queries keyed on `ProjectInput` — already shared by construction, doesn't need the loader.

**Why this matters:** The asymmetric-discovery bug class is the load-bearing failure mode of having two parallel reimplementations of "load a smelt project from disk". Two real instances: the LSP shipping without `functions/` discovery (VSCode showed `unknown-smelt-fn` while the CLI test suite was green), and the LSP not calling `set_loader_file` (so `smelt.config.load_yaml` failed silently in the editor only). Routing every consumer through the same loader makes this class structurally impossible. See `docs/specs/architecture.md` → "Workspace loading parity rule (CLI ↔ LSP)" for the normative spec.

**The rule in practice:**
- **DO** add new eager-discovery steps to `smelt_core::workspace::load_workspace`.
- **DO** route new lazy-discovery steps through Salsa queries keyed on `ProjectInput`.
- **DON'T** walk the filesystem from inside `Backend::initialize` or from CLI commands directly — call `load_workspace` instead.
- **DON'T** add a sibling discovery helper that only one side calls.

The standing CI gate is `cargo test -p smelt-lsp --test example_workspaces`, which drives the real `Backend` against every non-broken example workspace and asserts zero diagnostics. Always run this when touching LSP startup, CLI discovery, or `smelt-core::workspace`.

### Project Isolation Rule

**A workspace folder may contain multiple smelt projects. Each project is a closed resolution scope: a `smelt.<path>` reference inside project A resolves only against entities declared inside project A.**

This is an architectural invariant. `find_smelt_projects` discovers projects recursively from any workspace folder root (the LSP loads a monorepo with `examples/web_analytics/`, `examples/functions_demo/`, etc. all in one VSCode window). Same-name collisions across projects are independent, not errors — `smelt.functions.sessionize` in `web_analytics` and `smelt.functions.sessionize` in `functions_demo` are different functions with different signatures, and each call site sees only its own project's definition.

**Why this matters:** Resolvers that walk `workspace.files(db)` flat (no project filter) leak signatures between projects. The bug class first surfaced as a spurious `Missing required argument` on `examples/web_analytics/models/silver/sessions.sql` when VSCode opened the entire worktree — `resolve_function("sessionize")` returned `functions_demo`'s signature (alphabetically first), which has different parameter names. CLI test suites missed it because each `--test example_diagnostics` case ingests one project at a time. See `docs/specs/architecture.md` → "Project isolation rule" for the normative spec.

**The rule in practice:**
- **DO** thread `ProjectInput` through any Salsa query that takes `workspace: Workspace` and walks `workspace.files(db)` to resolve a name.
- **DO** derive the project from the file under analysis: `source_file.project_root(db)` → `find_project(workspace, root)` → `ProjectInput`.
- **DON'T** add a workspace-flat resolver helper; if you need cross-project information later, make it opt-in (future `dependencies:` declaration in `smelt.yml`).
- **DON'T** assume a workspace folder is one project — `find_smelt_projects` may return 0..N projects.

The standing CI gate is a multi-project case in `cargo test -p smelt-lsp --test example_workspaces` that opens the entire `examples/` directory as one workspace folder.

### Run Pipeline Parity Rule (CLI ↔ UI)

**The compile + execute pipeline lives in exactly one place: `smelt-runtime`. Both `smelt-cli`'s `commands/run.rs` and `smelt-ui`'s `run_manager.rs` consume it through a single `execute_project(request, reporter)` entry point.**

This covers: the selection/filter pass (resolve selectors, drop tests, drop `.gen` generators, expand emitted models), the compile pipeline (`SqlCompiler` with all four emitters, ephemeral inlining, `apply_type_casts`, `inject_time_filter`, `build_fn_body_map`), the pre-execution diagnostic gate (`UnknownSmeltFn` etc.), and the per-model execute loop (batch dispatch, `RunManifest`, intervals). Consumer crates contribute only surface concerns — argument parsing, stdout/HTTP serialization, and `RunReporter` implementations.

**Why this matters:** Incidents trace to two failure modes at different layers. **Mode A** (a consumer reimplements *analysis* logic instead of calling the shared analysis layer) is what the two rules above address — LSP `functions/`-discovery miss, `set_loader_file` miss, flat-resolver project leak. **Mode B** (a consumer reimplements *compile or execute* logic because there is no shared layer to call) is what this rule addresses — UI executing `materialization: test` models (today's `unreachable!` panic), UI silently passing `smelt.fn.*` calls through unexpanded because its `PrintContext` had `smelt_fn: None`, UI skipping `apply_type_casts` and ephemeral inlining, UI not expanding `*.gen.sql` generators. Layered single-ownership closes both modes. See `docs/specs/architecture.md` → "Run pipeline parity rule (CLI ↔ UI)" for the normative spec.

**The rule in practice:**
- **DO** place new lifecycle logic at the *lowest* layer that needs it. LSP-shared (parsing, analysis, type inference, diagnostics, workspace discovery, planning) → `smelt-parser` / `smelt-db` / `smelt-core` / `smelt-planner`. CLI+UI-only (compile, execute, manifests, selection/filter) → `smelt-runtime`. Surface concerns → consumer crate.
- **DO** make `smelt-runtime`'s internal helpers (`SqlCompiler`, emitter factories, `PrintContext` constructors, `inject_time_filter`) `pub(crate)` so consumers cannot construct a `CompiledModel` half-way.
- **DON'T** add a parallel compile or execute helper inside `smelt-cli` or `smelt-ui`. If `smelt-runtime` doesn't expose the shape you need, change `smelt-runtime`.
- **DON'T** move analysis logic *up* into `smelt-runtime`. Diagnostic checks, type inference, schema extraction, and workspace ingest stay in `smelt-db` / `smelt-core` so the LSP continues to consume them and a future `smelt-language-service` extraction (for UI in-browser editing) remains mechanical.

The standing CI gate is `cargo test -p smelt-runtime --test execute_parity`, which runs the same fixture project through both `smelt-cli` and `smelt-ui` entry points and asserts identical model outputs, manifest contents, and selection sets. Always run this when touching the compile pipeline, the execute loop, or either consumer's run path.

### Diagnostic Range Encoding Rule

**Diagnostics carry `rowan::TextRange` (byte-offset) values internally. Conversion to `(line, column)` form happens exactly once, at the boundary between smelt's analysis layer and an external consumer, backed by a per-file `line_index::LineIndex`.**

This covers every `Diagnostic` in `smelt-db`, every Salsa accumulator that emits a `Diagnostic`, and every body-offset helper that shifts diagnostic ranges across frontmatter boundaries. The LSP boundary converter lives in `smelt-lsp::diagnostics_boundary::BoundaryConverter` (encoding-aware: UTF-16 or UTF-8 per negotiated `positionEncodingKind`). The CLI boundary converter lives in `smelt-cli::diagnostics_terminal::TerminalConverter`. The UI JSON boundary uses `line_index::LineIndex` directly in `smelt-ui::build`. The helpers `offset_to_position` and `text_range_to_range` no longer exist in `smelt-parser` — they live only inside `smelt-lsp::diagnostics_boundary` for non-diagnostic LSP protocol surface (goto-definition, completions, rename). See `docs/specs/architecture.md` → "Diagnostic range encoding rule" for the normative spec.

**The rule in practice:**
- **DO** carry `rowan::TextRange` through every `Diagnostic`, every analysis function, and every Salsa query that returns positions.
- **DO** construct `LineIndex` once per file at the boundary and reuse it across all diagnostics for that file.
- **DO** consult the negotiated `positionEncodingKind` in the LSP boundary converter (defaults to UTF-16).
- **DON'T** convert `TextRange` to `(line, column)` inside `smelt-db`, `smelt-parser`, or any other analysis crate.
- **DON'T** store `Position` / `Range` fields on `Diagnostic` or any analysis-layer intermediate. `lsp_types::Range` is produced only at the LSP boundary.

The standing CI gate is `cargo test -p smelt-lsp --test position_encoding`, which verifies correct diagnostic positions for both UTF-8 and UTF-16 encoding negotiation, including non-ASCII text. Run `rg 'offset_to_position|text_range_to_range' crates/smelt-db/src/ crates/smelt-parser/src/` to confirm no analysis-crate violations were reintroduced.

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
7. **Run `cargo clippy --all-targets` and fix all warnings**
8. Run `cargo build` and `cargo test` to ensure everything compiles and passes
9. **Run `cargo test -p smelt-cli --test example_diagnostics`** to verify examples have no diagnostics via the Salsa-direct path
10. **Run `cargo test -p smelt-lsp --test example_workspaces`** to verify examples have no diagnostics via the real LSP backend (catches asymmetric-discovery bugs the Salsa-direct test misses)
11. Update docs/ROADMAP.md with completion status and date
12. **Commit** with descriptive message (includes ROADMAP.md update)

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

## License

MIT License - Copyright (c) 2025 Andrew Browne
