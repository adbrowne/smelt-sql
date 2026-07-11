//! W3·P1 self-test: verifies that `seed_source_table` materialises the source
//! table with the expected row count on every target in `targets_to_run()`.
//!
//! With `SPARK_CONNECT_URL` unset the test covers DuckDB only.
//! With `SPARK_CONNECT_URL` set AND `--features spark`, it also covers Spark.

mod common;

use common::{
    count_table_rows, seed_source_table, sessions_arrow_schema, sessions_record_batch,
    stage_dual_workspace, targets_to_run, TargetKind,
};
use tempfile::TempDir;

const EXPECTED_ROWS: usize = 3;

#[test]
#[cfg(feature = "duckdb")]
fn seed_source_table_self_test() {
    let tmp = TempDir::new().unwrap();
    let warehouse = tmp.path().join("warehouse");
    std::fs::create_dir_all(&warehouse).unwrap();

    for kind in targets_to_run() {
        let proj = stage_dual_workspace(&tmp, "seed_test", &[], &warehouse);

        // Connection params derived from targets_yaml conventions:
        //   DuckDB → target/dev.duckdb, schema main
        //   Spark  → SPARK_CONNECT_URL env, schema smelt_w1, warehouse dir
        let db_path = proj.join("target/dev.duckdb");

        let (table_fqn, schema, table_name) = match &kind {
            TargetKind::DuckDb => ("main.sources_raw_sessions", "main", "sources_raw_sessions"),
            TargetKind::Spark => (
                "smelt_w1.sources_raw_sessions",
                "smelt_w1",
                "sources_raw_sessions",
            ),
        };

        seed_source_table(
            &kind,
            &db_path,
            &warehouse,
            table_fqn,
            sessions_arrow_schema(),
            sessions_record_batch(),
        );

        let count = count_table_rows(&kind, &db_path, &warehouse, schema, table_name);
        assert_eq!(
            count, EXPECTED_ROWS,
            "{kind:?}: expected {EXPECTED_ROWS} seeded rows, got {count}"
        );
    }
}
