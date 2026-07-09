#![cfg(feature = "duckdb")]
//! Property-discovery research loop — Link-C in-process test target.
//!
//! `EXPERIMENTAL(property-discovery): disposable`. See
//! `docs/research/20260705-property-discovery-loop.md` (design) and
//! `docs/plans/20260705-property-discovery-loop.md` (plan). This target hosts
//! one module per resolved catalog cell, each a disposable probe built on
//! top of the graduated `smelt-maintenance-testkit` dev-dependency crate:
//! the in-process real-planner harness
//! (`smelt_maintenance_testkit::link_c_harness`), the single model-SQL
//! catalogue (`smelt_maintenance_testkit::model_shapes`), the Link-C oracle
//! (`smelt_maintenance_testkit::oracle`), and the run-schedule
//! generator/driver (`smelt_maintenance_testkit::run_schedule`) — see
//! `docs/specs/maintenance_plan.md` §References → Tests.

mod g_01_additive_agg_append_only;
mod g_02_additive_agg_redelivery;
mod g_03_idempotent_agg_append_only;
mod g_04_idempotent_min_mutable_snapshot;
mod g_05_join_enrichment_mutable_dimension;
mod g_06_left_join_null_preservation;
mod g_07_holistic_agg_append_only;
mod g_08_running_total_self_ref;
mod g_09_union_all_append_only;
mod g_11_self_ref_ambiguous_column;
mod g_12_keyed_merge_reprocessed_window;
mod p0_2_run_schedule;
mod p0_4_mutation_profile_selfcheck;
mod sc_1_correlated_exists;
mod sc_1b_column_name_collision;
mod sc_2_clocked_mutable_window_forward;
mod sc_4_stacked_frames;
mod sc_6_fd_over_union;
mod sc_7_cte_body_admission;
mod smoke;

// tracer_evolution/tracer_maintenance/tracer_propagation moved to
// `smelt-runtime/tests/` — they only depend on `duckdb` +
// `smelt_logical::maintenance::*`, not on this crate's `link_c_harness`
// (see `docs/research/20260705-refresh-as-maintenance-plan/08-code-placement.md`).
