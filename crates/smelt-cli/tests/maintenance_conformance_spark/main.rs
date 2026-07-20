#![cfg(feature = "spark")]
//! Spark twin of the standing generative maintenance-conformance gate
//! (`crates/smelt-cli/tests/maintenance_conformance/`;
//! `docs/plans/20260720-prod-w9-spark-conformance-twin.md` Phase 3;
//! `docs/specs/incremental_models.md` §"The equivalence invariant";
//! `docs/specs/multi_backend.md` §"Parity contract" — "Generative
//! equivalence coverage"). Drives the SAME append-only partition-grain
//! `ModelRecipe` pool, schedule driver, and S-restricted multiset oracle
//! (`smelt_maintenance_testkit`) the DuckDB leg uses, but against a live
//! Spark Connect/Delta backend — the backend under test is a parameter, not
//! a duplicated harness.
//!
//! Only compiled at all with `--features smelt-cli/spark` (this whole binary
//! is `#![cfg(feature = "spark")]`), so a bare `cargo test` never reaches it.
//! Each test additionally skips (green) when `SPARK_CONNECT_URL` is unset,
//! so `cargo test --features smelt-cli/spark` without a live server stays
//! green too — the *phase completion claim* requires a live-server run
//! (recorded separately), not this default-skip behaviour.
//!
//! Gated tier only (never per-PR): runtime cost of the generative pool over
//! Spark Connect makes unconditional per-PR runs impractical
//! (`multi_backend.md` §"CI tiering").

mod gate_keyed_spark;
mod gate_mixed_spark;
mod gate_spark;
mod harness_self_check_spark;
