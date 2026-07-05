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
mod oracle;
mod run_schedule;

mod g_01_additive_agg_append_only;
mod g_02_additive_agg_redelivery;
mod p0_2_run_schedule;
mod p0_4_mutation_profile_selfcheck;
mod sc_1_correlated_exists;
mod sc_2_clocked_mutable_window_forward;
mod smoke;
