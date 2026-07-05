// Consolidated property-test binary: shares one prop_helpers module and
// links once instead of 11 times. The two documented CI-gate property tests
// (type_property_tests, nullability_property_tests) stay as their own binaries
// so their `--test <name>` invocations keep working.
#![allow(dead_code, unused_imports, unused_macros)]

#[allow(dead_code)]
#[path = "../prop_helpers/mod.rs"]
mod prop_helpers;

mod aggregate_widening;
mod collation_tests;
mod maintenance_link_a;
mod prop_coercion_matrix;
mod prop_cte_types;
mod prop_nested_functions;
mod prop_setop_types;
mod prop_subquery_types;
mod prop_window_types;
mod struct_duckdb_validation;
mod ts_oracle;
mod type_conformance_tests;
