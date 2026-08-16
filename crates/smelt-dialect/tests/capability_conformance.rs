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
    let bigquery = BackendCapabilities::bigquery();

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

    // supports_native_ivm — matrix: all ✗ (no backend has native IVM today)
    cell!(duckdb, supports_native_ivm, false, "DuckDB");
    cell!(delta, supports_native_ivm, false, "Spark(Delta)");
    cell!(parquet, supports_native_ivm, false, "Spark(Parquet)");

    // supports_retraction — matrix: all ✗ (meaningful only alongside native IVM)
    cell!(duckdb, supports_retraction, false, "DuckDB");
    cell!(delta, supports_retraction, false, "Spark(Delta)");
    cell!(parquet, supports_retraction, false, "Spark(Parquet)");

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

    // supports_column_scoped_merge
    cell!(duckdb, supports_column_scoped_merge, true, "DuckDB");
    cell!(delta, supports_column_scoped_merge, true, "Spark(Delta)");
    cell!(
        parquet,
        supports_column_scoped_merge,
        false,
        "Spark(Parquet)"
    );

    // BigQuery. Every cell below was established by running the statement the flag
    // names against a live warehouse (`scripts/bigquery-probe.sh`) rather than read
    // from documentation, because the spec calls this table the honest matrix.
    cell!(bigquery, supports_qualify, true, "BigQuery");
    cell!(bigquery, supports_create_or_replace_table, true, "BigQuery");
    cell!(bigquery, supports_create_or_replace_view, true, "BigQuery");
    cell!(bigquery, supports_merge, true, "BigQuery");
    cell!(bigquery, supports_pivot, true, "BigQuery");
    cell!(bigquery, supports_date_literal, true, "BigQuery");
    cell!(bigquery, supports_concat_operator, true, "BigQuery");
    cell!(bigquery, supports_array_literal, true, "BigQuery");
    cell!(bigquery, supports_transactional_ddl, true, "BigQuery");
    cell!(bigquery, supports_double_colon_cast, false, "BigQuery");
    cell!(bigquery, supports_trailing_commas, true, "BigQuery");
    cell!(bigquery, supports_insert_overwrite, false, "BigQuery");
    // BigQuery *does* accept CREATE MATERIALIZED VIEW with incremental refresh, so
    // this cell is false for an implementation reason rather than a warehouse one:
    // `true` obliges smelt to emit the native maintained object, and that emission
    // path does not exist. Recorded in `multi_backend.md` §Known Divergences.
    cell!(bigquery, supports_native_ivm, false, "BigQuery");
    cell!(bigquery, supports_retraction, false, "BigQuery");
    cell!(bigquery, supports_struct_field_ddl, true, "BigQuery");
    // No USING clause exists in GoogleSQL — `ALTER COLUMN ... SET DATA TYPE ... USING`
    // is a syntax error, and the bare form only permits assignable widenings.
    cell!(bigquery, supports_alter_column_using, false, "BigQuery");
    cell!(bigquery, supports_nested_array_ddl, true, "BigQuery");
    cell!(bigquery, supports_merge_schema_write, false, "BigQuery");
    cell!(bigquery, supports_column_mapping, true, "BigQuery");
    // First backend to report native pipe support; the printer path behind this
    // flag has never been exercised before.
    cell!(bigquery, supports_pipe_syntax, true, "BigQuery");
    cell!(bigquery, requires_schema_init, true, "BigQuery");
    cell!(bigquery, supports_column_scoped_merge, true, "BigQuery");
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
        supports_native_ivm: _,
        supports_retraction: _,
        supports_struct_field_ddl: _,
        supports_alter_column_using: _,
        supports_nested_array_ddl: _,
        supports_merge_schema_write: _,
        supports_column_mapping: _,
        supports_pipe_syntax: _,
        requires_schema_init: _,
        supports_column_scoped_merge: _,
    } = BackendCapabilities::duckdb();
    // Adding a field to BackendCapabilities without listing it here is a compile error.
    // When that happens: add the field above, add it to every_flag_matches_matrix(),
    // and update docs/specs/multi_backend.md §Surface capability matrix.
}
