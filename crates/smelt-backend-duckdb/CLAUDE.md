# crates/smelt-backend-duckdb/CLAUDE.md

DuckDB backend implementation — wraps a `duckdb::Connection` in `Arc<Mutex<>>` and implements the `smelt-backend::Backend` async trait via `tokio::task::spawn_blocking`.

## How to test

```bash
# Unit tests (no feature flag needed when system DuckDB is configured)
cargo test -p smelt-backend-duckdb

# Bundled DuckDB (compiles from source — slow first build)
cargo test -p smelt-backend-duckdb --features bundled
```

See root `CLAUDE.md` §"Build and Test (System DuckDB)" for the `DUCKDB_LIB_DIR` setup. System DuckDB is the default and avoids the long C++ compile.

## Gotchas

- **`bundled` feature flag.** The crate exposes a `bundled` feature that maps to `duckdb/bundled`. Do not enable it by default — the workspace default is system DuckDB. When testing the bundled path explicitly, pass `--features bundled`.
- **`spawn_blocking` pattern.** Every `async fn` in `DuckDbBackend` wraps synchronous DuckDB calls in `tokio::task::spawn_blocking` because `duckdb::Connection` is not `Send + Sync`. The `Arc<Mutex<Connection>>` clone is moved into the closure.
- **Arrow type mapping.** `arrow_type_to_duckdb_ddl` maps Arrow `DataType` to DuckDB DDL strings for seed loading. The supported set is: `BOOLEAN`, `INTEGER`, `BIGINT`, `DECIMAL(p≤18, s≤4)`, `DOUBLE`, `DATE`, `TIMESTAMP` (no TZ), `VARCHAR`. Unsupported types return `Err` — callers in `smelt-runtime` surface these as `BackendError`.
- **Schema creation.** `DuckDbBackend::new` always ensures the target schema exists (`CREATE SCHEMA IF NOT EXISTS`). Each project gets its own schema (named after the project).
