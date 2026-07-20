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
//!
//! **Run with `-- --test-threads=1`.** Every recipe's `model_name` is
//! deterministic (derived from its construct/route, not per-test-run-unique
//! — `smelt_maintenance_testkit::recipe`'s own convention), and the Spark/
//! Delta warehouse (`crate::recipe::SPARK_CONFORMANCE_SCHEMA`) persists
//! across the whole binary rather than getting a fresh temp file per case
//! the way the DuckDB leg does. Two tests racing on the same physical Delta
//! table (or even two sequential tests in the same process without a clean
//! interval — observed as a `SIGSEGV` in the underlying PySpark/py4j bridge
//! during Phase 5 development) is a real hazard this binary has not yet
//! removed; serializing is the current mitigation. Tracked for a proper fix
//! (e.g. per-test schema/table namespacing) as follow-up, not blocking this
//! plan's phases.

mod dags_spark;
mod feed_spark;
mod gate_composed_spark;
mod gate_keyed_spark;
mod gate_mixed_spark;
mod gate_spark;
mod harness_self_check_spark;
mod pinned_spark;
