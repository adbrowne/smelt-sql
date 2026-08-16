//! Dev-only Link-C schedule/oracle harness + model-shape catalogue for
//! maintenance-plan equivalence testing (`docs/specs/incremental_models.md`
//! §References → Tests).
//!
//! This crate is **not** a production dependency of anything — `publish =
//! false`, and it is wired only as a `dev-dependency` of consumer crates'
//! test targets (see root `CLAUDE.md` §"Maintenance-plan purity": the
//! maintenance plan itself is pure data produced in `smelt-logical`; this
//! crate exists purely to *exercise* that plan end-to-end against a real
//! DuckDB backend, never to compute it).
//!
//! Graduated out of `smelt-cli`'s `tests/property_discovery/` research
//! harness (`docs/research/20260705-property-discovery-loop.md`,
//! `docs/research/20260705-refresh-as-maintenance-plan/08-code-placement.md`
//! §3, M3) once the Link-C schedule suite over [`model_shapes`] became a
//! standing equivalence gate rather than a disposable research probe. The
//! per-cell probe test modules that consume this crate's API remain in
//! `smelt-cli/tests/property_discovery/` and stay tagged disposable/
//! experimental (see `.claude/scripts/property-experimental-gate.sh`) — only
//! the shared harness pieces graduated.
//!
//! - [`link_c_harness`] — drives smelt's real run pipeline
//!   (`smelt_runtime::execute_project`) in-process over a temp DuckDB.
//! - [`oracle`] — the Link-C multiset-equality oracle (`EXCEPT ALL` both
//!   directions).
//! - [`recipe`] — `ModelRecipe`, the typed proptest value generating models
//!   as data over the partition-grain append-only construct pool
//!   (`docs/plans/20260712-generative-maintenance-conformance.md` Phase 1).
//! - [`render`] — renders a `ModelRecipe` into model/source/oracle SQL and a
//!   staged project.
//! - [`verdict`] — classifies a staged recipe through the *real* maintenance
//!   derivation (`smelt_db::maintenance_plan_report` + `file_diagnostics`)
//!   into `Admitted`/`Refused`, plus the adversarial leaf pool and the
//!   over-refusal ledger
//!   (`docs/plans/20260712-generative-maintenance-conformance.md` Phase 2).
//! - [`schedule_gen`] — schema-generic row/schedule generation over a
//!   [`recipe::ModelRecipe`]'s own source shape
//!   (`docs/plans/20260712-generative-maintenance-conformance.md` Phase 3).
//! - [`s_tracker`] — the S-tracker: records `(window, source snapshot)` per
//!   run and derives `S_k`, the append-only pool's S-restricted oracle
//!   baseline (`docs/plans/20260712-generative-maintenance-conformance.md`
//!   Phase 3); Phase 4 extends it with per-window outstanding-dimension-
//!   mutation bookkeeping for mixed (fact + mutable dimension) models.
//! - [`oracle_modes`] — the `OracleMode` enum a mixed model selects between
//!   (S-restricted vs settled-point,
//!   `docs/plans/20260712-generative-maintenance-conformance.md` Phase 4).
//! - [`probes`] — plan-claim probes: `CaseContext`/`ProbeOutcome`, the
//!   direct runtime checks that a derived plan claim actually holds beyond
//!   end-state equivalence alone (scan-clamp/compiled-SQL consistency,
//!   write-window containment, technique interchangeability), plus the
//!   skip-accounting `ReachabilityReport` fold
//!   (`docs/plans/20260712-generative-maintenance-conformance.md` Phase 7).
//! - [`dag`] — generated 2-3 node DAGs (chain, diamond, payload-leak pair)
//!   wiring [`recipe::ModelRecipe`]-style nodes together via `smelt.ref()`,
//!   for the propagation-sufficiency/backward-resolution suite in
//!   `smelt-cli`'s `tests/maintenance_conformance/dags.rs`
//!   (`docs/plans/20260712-generative-maintenance-conformance.md` Phase 10).
//! - [`feed`] — the `SimulatedChangeFeed` step family for `change_feed`-
//!   declared sources: base-table mutation + staged `(op, key, payload,
//!   seq)` feed-table bookkeeping, gated retraction/tombstone steps, and the
//!   `change_feed` YAML rendering the recompute-only admission gate stages
//!   against (`docs/plans/20260712-generative-maintenance-conformance.md`
//!   Phase 8).

/// The real target backend name for `config`'s first declared target
/// (`smelt.yml` `targets.*.type`, lower-cased to the vocabulary
/// [`smelt_db::queries::maintenance::state_availability_for`] matches) —
/// the same "first declared target" convention
/// `smelt-cli::commands::explain` uses for an offline, no-model-override
/// resolution. Every staged conformance project this crate builds declares
/// exactly one target, so [`verdict::classify`]/[`dag::classify_node`] see
/// the SAME plan a live run against that project's declared backend would
/// (`docs/specs/state.md` §"The degradation contract") rather than the
/// DuckDB-optimistic ideal every call site assumed before backend-aware
/// resolution landed.
pub fn dialect_name_for_config(config: &smelt_core::config::Config) -> &'static str {
    match config
        .targets
        .values()
        .next()
        .and_then(|t| t.backend_type().ok())
    {
        Some(smelt_core::config::BackendType::DuckDB) => "duckdb",
        Some(smelt_core::config::BackendType::Spark) => "spark",
        None => "duckdb",
    }
}

pub mod dag;
pub mod feed;
pub mod link_c_harness;
pub mod oracle;
pub mod oracle_modes;
pub mod probes;
pub mod recipe;
pub mod render;
pub mod s_tracker;
pub mod schedule_gen;
pub mod verdict;
