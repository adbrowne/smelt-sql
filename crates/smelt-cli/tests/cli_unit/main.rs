// Consolidated light (non-build) CLI tests: link once instead of ~24 times.
// (web_analytics_incremental_classification is a documented CI gate, so it
// stays as its own binary.)
#![allow(dead_code, unused_imports, unused_macros)]

mod as_struct_capability_tests;
mod as_struct_emission_tests;
mod broken_function_diagnostics;
mod cumulative_diagnostics;
mod cumulative_equivalence;
mod docs_json_output;
mod docs_markdown_output;
mod emitted_name_collision;
mod ephemeral_seed;
mod explain_json_output;
mod incremental_run_window;
mod incremental_test;
mod multi_engine_test;
mod path_form_compile;
mod path_form_e2e;
mod planner_test;
mod retail_analytics_validation;
mod seed_loading;
mod source_guard_and_name_override;
mod subdir_seed_resolution;
mod test_workspace_validation;
mod type_command_function_returns;
mod web_analytics_pushdown;
mod web_analytics_source_bounds;
