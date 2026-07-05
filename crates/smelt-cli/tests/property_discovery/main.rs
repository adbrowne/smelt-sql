#![cfg(feature = "duckdb")]
//! Property-discovery research loop — Link-C in-process test target.
//!
//! `EXPERIMENTAL(property-discovery): disposable`. See
//! `docs/research/20260705-property-discovery-loop.md` (design) and
//! `docs/plans/20260705-property-discovery-loop.md` (plan). This target hosts
//! the in-process real-planner harness (`link_c_harness`), the single model-SQL
//! catalogue (`model_shapes`), and one module per resolved catalog cell.

mod link_c_harness;
mod model_shapes;
mod run_schedule;

mod p0_2_run_schedule;
mod smoke;
