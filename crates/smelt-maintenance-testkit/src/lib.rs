//! Dev-only Link-C schedule/oracle harness + model-shape catalogue for
//! maintenance-plan equivalence testing (`docs/specs/maintenance_plan.md`
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
//! - [`model_shapes`] — the single readable catalogue of model shapes the
//!   equivalence suite exercises.
//! - [`oracle`] — the Link-C multiset-equality oracle (`EXCEPT ALL` both
//!   directions).
//! - [`run_schedule`] — the run-schedule generator + driver, including
//!   between-run source mutation (append/update/delete).
//! - [`recipe`] — `ModelRecipe`, the typed proptest value generating models
//!   as data over the partition-grain append-only construct pool
//!   (`docs/plans/20260712-generative-maintenance-conformance.md` Phase 1).
//! - [`render`] — renders a `ModelRecipe` into model/source/oracle SQL and a
//!   staged project.

pub mod link_c_harness;
pub mod model_shapes;
pub mod oracle;
pub mod recipe;
pub mod render;
pub mod run_schedule;
