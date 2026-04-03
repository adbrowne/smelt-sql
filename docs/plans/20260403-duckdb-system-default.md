# Plan: Default to System DuckDB Instead of Bundled

**Date**: 2026-04-03
**Research**: `docs/research/2026-04-03-duckdb-bundled-builds.md`
**Status**: Complete

## Overview

Bare `cargo build`/`cargo test` commands trigger a ~5-10 minute C++ compilation of DuckDB because all four DuckDB-consuming crates default to the `bundled` feature. This is problematic during Claude Code sessions where agents sometimes run bare cargo commands despite CLAUDE.md specifying the `--no-default-features` incantation.

The fix flips the defaults so system DuckDB is used by default, and bundled mode requires explicit opt-in. This eliminates the need for `--no-default-features` entirely.

## Current State

Four crates default to bundled DuckDB:

- `crates/smelt-backend-duckdb/Cargo.toml:27` — `default = ["bundled"]`
- `crates/smelt-cli/Cargo.toml:67` — `default = ["duckdb", "bundled-duckdb"]`
- `crates/smelt-ui/Cargo.toml:52` — `default = ["duckdb", "bundled-duckdb"]`
- `crates/smelt-db/Cargo.toml:25` — `default = ["bundled-duckdb"]`

CLAUDE.md documents two build modes with the system-DuckDB mode requiring verbose flags:
```bash
cargo build --no-default-features --features smelt-cli/duckdb,smelt-ui/duckdb
```

## Desired End State

- `cargo build` uses system `libduckdb.so` (fast, no C++ compilation)
- `cargo test` uses system `libduckdb.so`
- `cargo clippy --all-targets` uses system `libduckdb.so`
- Bundled mode available via explicit `--features bundled-duckdb` for CI/portability
- CLAUDE.md simplified — bare cargo commands are the recommended path

## What We're NOT Doing

- Removing the bundled feature entirely (still needed for CI and environments without system DuckDB)
- Changing the workspace-level `duckdb` dependency
- Modifying any Rust source code

## Implementation Phases

### Phase 1: Flip Feature Defaults

**Files to modify**:
- `crates/smelt-backend-duckdb/Cargo.toml` — change default features
- `crates/smelt-cli/Cargo.toml` — change default features
- `crates/smelt-ui/Cargo.toml` — change default features
- `crates/smelt-db/Cargo.toml` — change default features

**Changes**:

1. In `crates/smelt-backend-duckdb/Cargo.toml` L27, change:
   ```toml
   # Before
   default = ["bundled"]
   # After
   default = []
   ```

2. In `crates/smelt-cli/Cargo.toml` L67, change:
   ```toml
   # Before
   default = ["duckdb", "bundled-duckdb"]
   # After
   default = ["duckdb"]
   ```

3. In `crates/smelt-ui/Cargo.toml` L52, change:
   ```toml
   # Before
   default = ["duckdb", "bundled-duckdb"]
   # After
   default = ["duckdb"]
   ```

4. In `crates/smelt-db/Cargo.toml` L25, change:
   ```toml
   # Before
   default = ["bundled-duckdb"]
   # After
   default = []
   ```

**Verification**:
- [x] `cargo build` completes without compiling DuckDB C++ (check for `Compiling duckdb-loadable-macros` or `Compiling libduckdb-sys` build step — should NOT appear)
- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --all-targets` (no warnings)
- [x] `cargo test` (all pass)

### Phase 2: Update CLAUDE.md

**Files to modify**:
- `CLAUDE.md` — simplify build commands

**Changes**:

1. In the "Build and Test (System DuckDB - Recommended)" section, simplify commands to bare cargo commands:
   ```bash
   cargo build
   cargo fmt --all
   cargo clippy --all-targets
   cargo test
   cargo test -p smelt-cli --test example_diagnostics
   cargo run -p smelt-lsp
   ```

2. In the "Build and Test (Bundled DuckDB)" section, update to show the explicit feature flags needed:
   ```bash
   cargo build --features smelt-cli/bundled-duckdb,smelt-ui/bundled-duckdb
   cargo test --features smelt-cli/bundled-duckdb,smelt-ui/bundled-duckdb,smelt-db/bundled-duckdb
   cargo clippy --all-targets --features smelt-cli/bundled-duckdb,smelt-ui/bundled-duckdb
   ```

3. Update the property test section to use bare commands:
   ```bash
   cargo test -p smelt-db --test type_property_tests
   ```

**Verification**:
- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --all-targets` (no warnings)
- [x] `cargo test` (all pass)
- [x] `cargo test -p smelt-cli --test example_diagnostics` (passes)

## Testing Strategy

1. Run `cargo build` and confirm no C++ compilation occurs (look for `libduckdb-sys` compile step)
2. Run full test suite with `cargo test`
3. Run `cargo test -p smelt-db --test type_property_tests` specifically (this was the dev-dependency trap)
4. Run `cargo test -p smelt-cli --test example_diagnostics` (example workspace validation)

## Risks & Mitigations

| Risk | Mitigation |
|------|-----------|
| System DuckDB not installed → build fails | Clear error from `libduckdb-sys` pointing to missing library; CLAUDE.md setup section already documents installation |
| `DUCKDB_LIB_DIR` not set → link failure | `libduckdb-sys` falls back to pkg-config and standard library paths (`/usr/local/lib`); CLAUDE.md documents the env var |
| CI environments lack system DuckDB | CI should use `--features bundled-duckdb` explicitly (no CI config exists yet, so no change needed) |
