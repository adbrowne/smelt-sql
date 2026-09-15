//! Trino twin of the standing generative maintenance-conformance gate
//! (`crates/smelt-cli/tests/maintenance_conformance/`;
//! `docs/outcomes/20260913-trino-incremental/phases/08-plan.md`;
//! `docs/specs/incremental_models.md` §"The equivalence invariant";
//! `docs/specs/multi_backend.md` §"Generative equivalence coverage"). Drives
//! the SAME recipe pools, schedule driver, and S-restricted multiset oracle
//! (`smelt_maintenance_testkit`) the DuckDB, Spark, and BigQuery legs use,
//! against a live Trino/Iceberg coordinator — the backend under test is a
//! parameter, not a duplicated harness. Every `#[test]` here is a thin
//! wrapper: staging/drive/assert logic lives in
//! `smelt_maintenance_testkit::families::*` (`backend.rs` supplies only the
//! Trino-specific facts — per-case schema, corruption SQL, the no-session-
//! temp-view oracle relation — through `ConformanceBackend`).
//!
//! No feature gate on this binary at all: unlike the Spark/BigQuery twins
//! (`#![cfg(feature = "spark")]`/`#![cfg(feature = "bigquery")]`), Trino's
//! generative-conformance arm needs no optional client library — only a
//! runtime env gate. `cargo test -p smelt-cli --test maintenance_conformance_trino`
//! always compiles; each test skips (green) when `SMELT_TRINO_URL` is
//! unset, exactly like every other live-gated Trino suite in this crate.

mod backend;
mod gate_trino;
mod harness_self_check_trino;
