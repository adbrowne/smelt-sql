// Consolidated e2e CLI tests (most invoke a real `smelt build`). Linking one
// binary instead of ~38. Running all of them in parallel threads in one
// process was measured at ~150 MB peak RSS, so concurrent DuckDB builds are not
// a memory concern here. Documented CI-gate tests (example_diagnostics,
// ecommerce_execution, web_analytics_incremental_classification) stay as their
// own binaries so their `--test <name>` invocations keep working.
#![allow(dead_code, unused_imports, unused_macros)]

mod absent_schema_snapshot;
mod address_collision;
mod backbuild_cumulative_e2e;
mod build_summary_visibility;
mod cohort_count_acceptance;
mod combined_generators_e2e;
mod cross_midnight_rebase;
mod cumulative_classifier_gate;
mod declared_fact_probe_firing;
mod diagnostic_gate;
mod dry_run_visibility;
mod events_deduped_redelivery_equivalence;
mod example_builds;
mod fn_body_wiring;
mod fn_incremental_ts_e2e;
mod functions_e2e;
mod incremental_idempotency;
mod incremental_nondeterministic_columns_e2e;
mod incremental_not_derivable;
mod incremental_refusal;
mod meta_columns_e2e;
mod meta_config_e2e;
mod meta_hofs_e2e;
mod meta_lists_e2e;
mod meta_workspace_e2e;
mod path_prefix_build;
mod per_partition_equivalence;
mod run_command_end_to_end;
mod schema_evolution_incremental;
mod schema_roundtrip;
mod scope_integration;
mod select_parse_gate;
mod selector_resolution;
mod show_plan;
mod since_upstream_composed_web_analytics;
mod smelt_shop_idempotency;
mod source_diagnostics;
mod source_pushdown_e2e;
mod test_cte_level;
mod test_decimal_compare;
mod test_inputs_key_form;
mod test_json_output;
mod two_layer_clamp;
mod ui_startup_generator_expansion;
mod verbose_flag;
mod version_flag;
mod web_analytics_backfill;
mod web_analytics_refactor_snapshot;
