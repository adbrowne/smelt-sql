//! Capability conformance: each `BackendCapabilities` constructor must match the
//! capability matrix in `docs/specs/multi_backend.md` §Surface.
//!
//! This is the standing drift gate required by the spec §Constraints. When a flag
//! changes, this test and the spec table change in the **same commit**.
//!
//! One cell was excluded from P1's assertions pending live verification (W6·P2):
//!   - `supports_struct_field_ddl` / spark_parquet  — RESOLVED: false (W6·P2 live test)
//!   - `supports_nested_array_ddl` / spark_delta    — RESOLVED: true (W7·P2 live Delta test)

use smelt_dialect::BackendCapabilities;

/// Assert every `BackendCapabilities` flag against the matrix in
/// `docs/specs/multi_backend.md` §Surface.
///
/// Row order follows the spec table. All cells are asserted — no provisional exclusions remain.
#[test]
fn every_flag_matches_matrix() {
    let duckdb = BackendCapabilities::duckdb();
    let delta = BackendCapabilities::spark_delta();
    let parquet = BackendCapabilities::spark_parquet();

    macro_rules! cell {
        ($caps:expr, $flag:ident, $expected:expr, $backend:literal) => {
            assert_eq!(
                $caps.$flag,
                $expected,
                "capability matrix mismatch: {} / {} — expected {} but constructor reports {}",
                stringify!($flag),
                $backend,
                $expected,
                $caps.$flag
            );
        };
    }

    // supports_qualify
    cell!(duckdb, supports_qualify, true, "DuckDB");
    cell!(delta, supports_qualify, false, "Spark(Delta)");
    cell!(parquet, supports_qualify, false, "Spark(Parquet)");

    // supports_create_or_replace_table
    cell!(duckdb, supports_create_or_replace_table, true, "DuckDB");
    cell!(
        delta,
        supports_create_or_replace_table,
        false,
        "Spark(Delta)"
    );
    cell!(
        parquet,
        supports_create_or_replace_table,
        false,
        "Spark(Parquet)"
    );

    // supports_create_or_replace_view
    cell!(duckdb, supports_create_or_replace_view, true, "DuckDB");
    cell!(delta, supports_create_or_replace_view, true, "Spark(Delta)");
    cell!(
        parquet,
        supports_create_or_replace_view,
        true,
        "Spark(Parquet)"
    );

    // supports_merge
    cell!(duckdb, supports_merge, true, "DuckDB");
    cell!(delta, supports_merge, true, "Spark(Delta)");
    cell!(parquet, supports_merge, false, "Spark(Parquet)");

    // supports_pivot
    cell!(duckdb, supports_pivot, true, "DuckDB");
    cell!(delta, supports_pivot, true, "Spark(Delta)");
    cell!(parquet, supports_pivot, true, "Spark(Parquet)");

    // supports_date_literal
    cell!(duckdb, supports_date_literal, true, "DuckDB");
    cell!(delta, supports_date_literal, false, "Spark(Delta)");
    cell!(parquet, supports_date_literal, false, "Spark(Parquet)");

    // supports_concat_operator
    cell!(duckdb, supports_concat_operator, true, "DuckDB");
    cell!(delta, supports_concat_operator, true, "Spark(Delta)");
    cell!(parquet, supports_concat_operator, true, "Spark(Parquet)");

    // supports_array_literal
    cell!(duckdb, supports_array_literal, true, "DuckDB");
    cell!(delta, supports_array_literal, false, "Spark(Delta)");
    cell!(parquet, supports_array_literal, false, "Spark(Parquet)");

    // supports_transactional_ddl
    cell!(duckdb, supports_transactional_ddl, true, "DuckDB");
    cell!(delta, supports_transactional_ddl, false, "Spark(Delta)");
    cell!(parquet, supports_transactional_ddl, false, "Spark(Parquet)");

    // supports_double_colon_cast
    cell!(duckdb, supports_double_colon_cast, true, "DuckDB");
    cell!(delta, supports_double_colon_cast, false, "Spark(Delta)");
    cell!(parquet, supports_double_colon_cast, false, "Spark(Parquet)");

    // supports_trailing_commas
    cell!(duckdb, supports_trailing_commas, true, "DuckDB");
    cell!(delta, supports_trailing_commas, false, "Spark(Delta)");
    cell!(parquet, supports_trailing_commas, false, "Spark(Parquet)");

    // supports_insert_overwrite
    cell!(duckdb, supports_insert_overwrite, false, "DuckDB");
    cell!(delta, supports_insert_overwrite, true, "Spark(Delta)");
    cell!(parquet, supports_insert_overwrite, true, "Spark(Parquet)");

    // supports_materialized_views — matrix: all ✗ (table fallback for every backend)
    cell!(duckdb, supports_materialized_views, false, "DuckDB");
    cell!(delta, supports_materialized_views, false, "Spark(Delta)");
    cell!(
        parquet,
        supports_materialized_views,
        false,
        "Spark(Parquet)"
    );

    // supports_struct_field_ddl
    cell!(duckdb, supports_struct_field_ddl, true, "DuckDB");
    cell!(delta, supports_struct_field_ddl, true, "Spark(Delta)");
    // Empirically verified W6·P2 (Spark 4.1.x, Parquet): ALTER TABLE ADD COLUMNS with
    // a qualified struct path is rejected; constructor and matrix both set to false.
    cell!(parquet, supports_struct_field_ddl, false, "Spark(Parquet)");

    // supports_alter_column_using
    cell!(duckdb, supports_alter_column_using, true, "DuckDB");
    cell!(delta, supports_alter_column_using, false, "Spark(Delta)");
    cell!(
        parquet,
        supports_alter_column_using,
        false,
        "Spark(Parquet)"
    );

    // supports_nested_array_ddl
    cell!(duckdb, supports_nested_array_ddl, true, "DuckDB");
    // Empirically resolved W7·P2: Delta ALTER on array-of-struct column succeeds → true
    cell!(delta, supports_nested_array_ddl, true, "Spark(Delta)");
    cell!(parquet, supports_nested_array_ddl, false, "Spark(Parquet)");

    // supports_merge_schema_write
    cell!(duckdb, supports_merge_schema_write, false, "DuckDB");
    cell!(delta, supports_merge_schema_write, true, "Spark(Delta)");
    cell!(parquet, supports_merge_schema_write, true, "Spark(Parquet)");

    // supports_column_mapping
    cell!(duckdb, supports_column_mapping, false, "DuckDB");
    cell!(delta, supports_column_mapping, true, "Spark(Delta)");
    cell!(parquet, supports_column_mapping, false, "Spark(Parquet)");

    // supports_pipe_syntax
    cell!(duckdb, supports_pipe_syntax, false, "DuckDB");
    cell!(delta, supports_pipe_syntax, false, "Spark(Delta)");
    cell!(parquet, supports_pipe_syntax, false, "Spark(Parquet)");

    // requires_schema_init
    cell!(duckdb, requires_schema_init, true, "DuckDB");
    cell!(delta, requires_schema_init, true, "Spark(Delta)");
    cell!(parquet, requires_schema_init, true, "Spark(Parquet)");
}

/// Exhaustiveness guard: destructuring all `BackendCapabilities` fields triggers a
/// compile error when a new field is added without updating this test and the matrix.
#[test]
fn all_fields_destructured() {
    let BackendCapabilities {
        supports_qualify: _,
        supports_create_or_replace_table: _,
        supports_create_or_replace_view: _,
        supports_merge: _,
        supports_pivot: _,
        supports_date_literal: _,
        supports_concat_operator: _,
        supports_array_literal: _,
        supports_transactional_ddl: _,
        supports_double_colon_cast: _,
        supports_trailing_commas: _,
        supports_insert_overwrite: _,
        supports_materialized_views: _,
        supports_struct_field_ddl: _,
        supports_alter_column_using: _,
        supports_nested_array_ddl: _,
        supports_merge_schema_write: _,
        supports_column_mapping: _,
        supports_pipe_syntax: _,
        requires_schema_init: _,
    } = BackendCapabilities::duckdb();
    // Adding a field to BackendCapabilities without listing it here is a compile error.
    // When that happens: add the field above, add it to every_flag_matches_matrix(),
    // and update docs/specs/multi_backend.md §Surface capability matrix.
}
