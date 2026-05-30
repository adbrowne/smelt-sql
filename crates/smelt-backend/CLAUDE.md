# crates/smelt-backend/CLAUDE.md

Abstract backend trait — the `Backend` async trait that all execution engines implement, plus `BackendError`, `ExecutionResult`, `MaterializationStrategy`, and `PartitionSpec`.

## How to test

```bash
cargo test -p smelt-backend
```

Most backend behaviour is tested via the concrete implementations (`smelt-backend-duckdb`) and the CLI integration tests.

## Gotchas

- **`Backend` is `async_trait`.** All methods are async; concrete implementations (DuckDB, Spark) must `#[async_trait]`. DuckDB wraps synchronous calls in `tokio::task::spawn_blocking`.
- **`BackendCapabilities` and `SqlDialect` are re-exported from `smelt-dialect`.** Capability checks (e.g. whether a backend supports `MERGE INTO`) go through `SqlDialect`; dialect-specific SQL generation lives in `smelt-dialect`, not here.
- **`PartitionRange` and `PartitionSpec`** are the types used to communicate which time partitions a backend should populate during an incremental run. Backends receive these from `smelt-runtime` — they do not decide the partition range themselves.
- **Error handling.** `BackendError` wraps query failures, connection failures, and DDL failures. Backends should produce specific error variants (not generic `anyhow`) so `smelt-runtime` can surface meaningful messages.

## Where things live

- `src/lib.rs` — `Backend` trait definition (all required methods)
- `src/error.rs` — `BackendError` and its variants
- `src/types.rs` — `ExecutionResult`, `MaterializationStrategy`, `PartitionRange`, `PartitionSpec`
