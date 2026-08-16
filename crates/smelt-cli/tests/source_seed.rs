//! W3·P1 self-test: verifies that `seed_source_table` materialises the source
//! table with the expected row count on every target in `targets_to_run()`.
//!
//! With `SPARK_CONNECT_URL` unset the test covers DuckDB only.
//! With `SPARK_CONNECT_URL` set AND `--features spark`, it also covers Spark.
//! With `SMELT_BQ_PROJECT`/`SMELT_BQ_ACCESS_TOKEN` set AND `--features
//! bigquery`, it also covers BigQuery.

mod common;

use common::{
    count_table_rows, drop_bq_dataset, seed_source_table, sessions_arrow_schema,
    sessions_record_batch, stage_dual_workspace, targets_to_run, TargetKind,
};
use tempfile::TempDir;

const EXPECTED_ROWS: usize = 3;

/// Names both the staged `bq:` target block and the dataset the loop reads back.
const LABEL: &str = "seed_test";

#[test]
#[cfg(feature = "duckdb")]
fn seed_source_table_self_test() {
    let tmp = TempDir::new().unwrap();
    let warehouse = tmp.path().join("warehouse");
    std::fs::create_dir_all(&warehouse).unwrap();

    for kind in targets_to_run(LABEL) {
        let proj = stage_dual_workspace(&tmp, LABEL, &[], &warehouse);

        // Connection params derived from targets_yaml conventions:
        //   DuckDB → target/dev.duckdb, schema main
        //   Spark  → SPARK_CONNECT_URL env, schema smelt_w1, warehouse dir
        let db_path = proj.join("target/dev.duckdb");

        //   BigQuery → the suite-scoped dataset carried by the target kind
        let table_name = "sources_raw_sessions";
        let schema = match &kind {
            TargetKind::DuckDb => "main".to_string(),
            TargetKind::Spark => "smelt_w1".to_string(),
            TargetKind::BigQuery { dataset } => dataset.clone(),
        };
        let table_fqn = format!("{schema}.{table_name}");

        seed_source_table(
            &kind,
            &db_path,
            &warehouse,
            &table_fqn,
            sessions_arrow_schema(),
            sessions_record_batch(),
        );

        let count = count_table_rows(&kind, &db_path, &warehouse, &schema, table_name);
        drop_bq_dataset(&kind);
        assert_eq!(
            count, EXPECTED_ROWS,
            "{kind:?}: expected {EXPECTED_ROWS} seeded rows, got {count}"
        );
    }
}
