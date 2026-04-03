# Research: DuckDB Bundled Builds During Claude Code Sessions

**Date**: 2026-04-03
**Topic**: Why DuckDB sometimes compiles from C++ source instead of using the system library
**Branch**: main
**Commit**: 22c1a19

## Summary

Multiple crates have `default = ["bundled-duckdb"]` or `default = ["bundled"]` in their feature flags. Any `cargo build`, `cargo test`, or `cargo clippy` command that omits `--no-default-features` will activate the bundled feature, causing DuckDB to compile from C++ source (~5-10 min). The CLAUDE.md instructions specify `--no-default-features --features smelt-cli/duckdb,smelt-ui/duckdb`, but a bare `cargo test` or `cargo build` will trigger the bundled build.

## Key Files

| File | Purpose | Key Lines |
|------|---------|-----------|
| `crates/smelt-backend-duckdb/Cargo.toml` | DuckDB backend crate | L27-28: `default = ["bundled"]`, `bundled = ["duckdb/bundled"]` |
| `crates/smelt-cli/Cargo.toml` | CLI crate | L67-69: `default = ["duckdb", "bundled-duckdb"]`, `bundled-duckdb = ["smelt-backend-duckdb/bundled", "duckdb/bundled"]` |
| `crates/smelt-ui/Cargo.toml` | UI crate | L52-54: `default = ["duckdb", "bundled-duckdb"]`, `bundled-duckdb = ["smelt-backend-duckdb/bundled"]` |
| `crates/smelt-db/Cargo.toml` | DB/Salsa crate | L25-27: `default = ["bundled-duckdb"]`, `bundled-duckdb = ["duckdb/bundled"]` |
| `Cargo.toml` (workspace) | Workspace root | L20: `duckdb = { version = "1.4.4" }` (no default bundled at workspace level) |

## The Problem: Feature Flag Defaults

### Crates with bundled defaults

All four DuckDB-consuming crates default to bundled mode:

1. **smelt-backend-duckdb**: `default = ["bundled"]` (L27)
2. **smelt-cli**: `default = ["duckdb", "bundled-duckdb"]` (L67)
3. **smelt-ui**: `default = ["duckdb", "bundled-duckdb"]` (L52)
4. **smelt-db**: `default = ["bundled-duckdb"]` (L25) - only for dev-dependencies (tests)

### How bundled gets activated

The `duckdb` crate has a `bundled` feature that compiles `libduckdb-sys` from C++ source using the bundled `duckdb.hpp`/`duckdb.cpp` amalgamation. When this feature is NOT active, `libduckdb-sys` looks for a system-installed `libduckdb.so` via `DUCKDB_LIB_DIR` or pkg-config.

**Correct command** (from CLAUDE.md):
```bash
cargo build --no-default-features --features smelt-cli/duckdb,smelt-ui/duckdb
```
This disables all defaults (including `bundled-duckdb`) and only enables the `duckdb` feature (without `bundled`).

**Problematic commands** (any of these trigger bundled build):
```bash
cargo build                    # uses all default features
cargo test                     # uses all default features  
cargo clippy --all-targets     # uses all default features
cargo test -p smelt-db         # smelt-db default includes bundled-duckdb
```

### The smelt-db dev-dependency trap

`smelt-db` has DuckDB only as a `dev-dependency` (for property tests), but its `default` features include `bundled-duckdb`. This means `cargo test -p smelt-db` without `--no-default-features` will trigger a bundled build even though DuckDB is only needed for tests.

## How Claude Code Sessions Trigger This

The CLAUDE.md contains correct system-DuckDB commands, but:

1. **Bare cargo commands**: If a Claude session runs `cargo test` or `cargo build` without the `--no-default-features` flag, defaults activate bundled mode.
2. **Single-crate testing**: `cargo test -p smelt-db --test type_property_tests` without `--no-default-features` activates `smelt-db`'s bundled default.
3. **CLAUDE.md also documents bundled mode**: The "Bundled DuckDB" section shows `cargo build` and `cargo test` as valid commands, which could be followed instead of the system-DuckDB section.

## Current Behavior

- Workspace `duckdb` dependency at L20 has no default `bundled` feature
- Each consuming crate opts into `bundled` via its own default features
- `--no-default-features` at the workspace level disables ALL crate defaults, requiring explicit `--features` to re-enable just the `duckdb` (non-bundled) features
- The `DUCKDB_LIB_DIR` environment variable is only consulted by `libduckdb-sys` when the `bundled` feature is NOT active

## Open Questions

1. **Should defaults be flipped?** If system DuckDB is the intended build mode, the defaults could be changed to NOT include bundled, requiring `--features bundled-duckdb` only when explicitly wanted. This would make bare `cargo build`/`cargo test` use the system library.
2. **Could a `.cargo/config.toml` help?** A workspace-level cargo config could potentially set default build flags, though cargo doesn't support default feature overrides in config files.
3. **Are there other Claude Code entry points?** Hook scripts, background agents, or worktree agents might run cargo commands without the correct flags.
