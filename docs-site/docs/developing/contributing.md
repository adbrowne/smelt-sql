# Contributing

This page covers how to build, test, and contribute to smelt. For a deep dive into the system design, see [Architecture](architecture.md).

---

## Prerequisites

- **Rust toolchain** -- Install via [rustup.rs](https://rustup.rs/). The project uses stable Rust.
- **Node.js 18+** -- Only required for VSCode extension development.
- **Python 3.10+** -- Only required for Python model features.

DuckDB is bundled automatically -- no system installation needed.

---

## Building

```bash
cargo build              # Build all crates (DuckDB bundled by default)
cargo build --release    # Optimized build
```

---

## Code Quality

Both of these must pass before committing:

```bash
cargo fmt --all            # Format all code
cargo clippy --all-targets # Lint -- must produce zero warnings
```

To check formatting without modifying files:

```bash
cargo fmt --all -- --check
```

---

## Testing

```bash
cargo test                 # Run all tests
cargo test -p smelt-parser # Test a specific crate
```

### Property-Based Tests

Property tests in `smelt-db` verify that smelt's type inference matches DuckDB's actual behavior. They generate random SQL expressions, run them through both smelt's type inference and DuckDB, and compare results.

```bash
# Run all property tests (256 cases + smoke tests)
cargo test -p smelt-db --test type_property_tests

# Run only the main property test
cargo test -p smelt-db --test type_property_tests prop_type_inference

# Deeper coverage (slow, for local use)
PROPTEST_CASES=1000 cargo test -p smelt-db --test type_property_tests prop_type_inference
```

When a property test fails:

1. Check if it is a known divergence -- add to `tests/prop_helpers/divergences.rs`
2. Check if types are compatible (e.g., Text vs Varchar, Decimal precision) -- add to `tests/prop_helpers/type_comparison.rs`
3. Otherwise fix the inference in `smelt-db/src/type_inference.rs`

---

## Example Workspaces

Several example projects are included for manual testing:

```bash
# User/event analytics pipeline (12 models, incremental)
cargo run -p smelt-cli -- run --project-dir examples/timeseries

# TPC-DS-based retail pipeline (25 models: staging/intermediate/marts)
cargo run -p smelt-cli -- run --project-dir examples/retail_analytics
```

Other example directories:

| Directory | Purpose |
|-----------|---------|
| `examples/broken/` | Intentionally broken models for testing error handling |
| `examples/test_workspace/` | Minimal workspace for VSCode/LSP integration testing |
| `examples/huge/` | Auto-generated 2000-model workspace for stress testing |

---

## VSCode Extension

The VSCode extension provides syntax highlighting, diagnostics, go-to-definition, and completions via the smelt LSP.

```bash
cd editors/vscode
npm install
npm run compile
```

To test in development mode, open the `editors/vscode` directory in VSCode and press **F5** to launch the Extension Development Host.

```bash
npm run package   # Package as .vsix
npm run watch     # Auto-recompile on changes
```

---

## Running the LSP

```bash
cargo run -p smelt-lsp
```

Configure your editor to use the LSP server binary, then open a project containing a `models/` directory. The `examples/test_workspace/` directory is useful for quick testing.

---

## Project Conventions

### Git Workflow

- For small or trivial changes, commit directly to `main`.
- For larger work, use a local branch, then merge to `main`.
- No pull requests required -- push to `main` after tests pass.

### Implementation Plans

Before starting non-trivial implementation work, create a plan document:

```
docs/plans/YYYYMMDD-short-name.md
```

For example: `docs/plans/20260321-planner-api.md`. Plans are committed alongside (or before) the implementation they describe.

### Roadmap Updates

After completing a phase or making an architectural decision, update `docs/ROADMAP.md`:

- Mark completed phases with a date
- Document deferred work with rationale
- Propose concrete next-step options

### Development Checklist

Before committing changes:

1. `cargo fmt --all`
2. `cargo clippy --all-targets` (zero warnings)
3. `cargo test`

---

## Project Structure

```
smelt-sql/
  crates/
    smelt-types/           # Shared SQL data types
    smelt-parser/          # Rowan CST parser
    smelt-core/            # Project config, model discovery
    smelt-db/              # Salsa incremental queries
    smelt-dialect/         # Dialect-aware SQL printer
    smelt-planner/         # Optimization rules
    smelt-backend/         # Execution trait
    smelt-backend-duckdb/  # DuckDB backend
    smelt-backend-spark/   # Spark backend
    smelt-cli/             # CLI binary
    smelt-lsp/             # LSP server
    smelt-state/           # Run state tracking
    smelt-ui/              # Web dashboard
    smelt-bench/           # Benchmarks
    smelt-datagen/         # Test data generation
    smelt-parser-compat/   # Parser compatibility tests
  editors/
    vscode/                # VSCode extension
  examples/
    timeseries/            # Analytics example project
    retail_analytics/      # TPC-DS-based example
    test_workspace/        # Minimal LSP test project
    huge/                  # 2000-model stress test
  docs/
    plans/                 # Implementation plans
```

For details on how these crates interact, see [Architecture](architecture.md).
