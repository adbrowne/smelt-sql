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
mod smoke;
mod tracer_evolution;
mod tracer_maintenance;
mod tracer_propagation;
