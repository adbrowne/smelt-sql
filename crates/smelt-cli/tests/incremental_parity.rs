//! W5·P3 — incremental DELETE+INSERT idempotency parity across backends.
//!
//! Exercises `Backend::delete_partitions` + `Backend::insert_into_from_query`
//! on both DuckDB and Spark.  Runs the same time window twice and asserts no
//! row duplication — the core DELETE+INSERT invariant.
//!
//! With `SPARK_CONNECT_URL` unset: DuckDB only (Spark path skips green).
//! With `SPARK_CONNECT_URL` set AND `--features spark`: also covers Spark.
//! With `SMELT_BQ_PROJECT`/`SMELT_BQ_ACCESS_TOKEN` set AND `--features
//! bigquery`: also covers BigQuery, where `insert_overwrite` is unavailable and
//! the scoped DELETE+INSERT emulation is the only path.

mod common;
#[cfg(feature = "spark")]
use common::spark_connect_url;
use common::{assert_table_parity, drop_bq_dataset, fetch_rows, targets_to_run, TargetKind};
use smelt_backend::PartitionRange;
use tempfile::TempDir;

const SPARK_SCHEMA: &str = "smelt_incr_p3";

/// Scopes this suite's BigQuery dataset, as `SPARK_SCHEMA` does for Spark.
const BQ_LABEL: &str = "incr_p3";

/// SQL to create the transaction source table (4 rows, 2 days × 2 users per day).
///
/// Uses UNION ALL instead of VALUES for compatibility across DuckDB and Spark SQL dialects.
fn create_source_sql(schema: &str) -> String {
    format!(
        "CREATE TABLE IF NOT EXISTS {schema}.txn_source AS \
         SELECT '2024-01-01' AS event_date, 'A' AS user_id, 100 AS amount \
         UNION ALL SELECT '2024-01-01', 'B', 200 \
         UNION ALL SELECT '2024-01-02', 'A', 150 \
         UNION ALL SELECT '2024-01-02', 'C', 300"
    )
}

/// Model SQL: aggregate amount by date+user for a time window [start, end).
fn window_sql(schema: &str, start: &str, end: &str) -> String {
    format!(
        "SELECT event_date, user_id, CAST(SUM(amount) AS BIGINT) AS total_amount \
         FROM {schema}.txn_source \
         WHERE event_date >= '{start}' AND event_date < '{end}' \
         GROUP BY event_date, user_id"
    )
}

/// Expected rows after materialising the 2024-01-01 window.
fn expected_rows() -> Vec<Vec<String>> {
    let mut rows = vec![
        vec!["2024-01-01".to_string(), "A".to_string(), "100".to_string()],
        vec!["2024-01-01".to_string(), "B".to_string(), "200".to_string()],
    ];
    rows.sort();
    rows
}

/// Prove DELETE+INSERT is idempotent on both DuckDB and Spark:
/// run the same time window twice and assert no row duplication.
///
/// **Red on Spark** if `delete_partitions` or `insert_into_from_query` diverges
/// from DuckDB behaviour (e.g. DELETE unsupported without Delta, wrong row count).
/// **DuckDB always green; Spark skips when `SPARK_CONNECT_URL` is unset.**
#[cfg(feature = "duckdb")]
#[test]
fn incremental_delete_insert_is_idempotent_on_both() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.duckdb");
    let warehouse = tmp.path().join("warehouse");
    std::fs::create_dir_all(&warehouse).unwrap();
    let rt = tokio::runtime::Runtime::new().unwrap();

    let window_start = "2024-01-01";
    let window_end = "2024-01-02";

    for kind in targets_to_run(BQ_LABEL) {
        let schema = match &kind {
            TargetKind::DuckDb => "main",
            TargetKind::Spark => SPARK_SCHEMA,
            TargetKind::BigQuery { dataset } => dataset.as_str(),
        };

        let partition = PartitionRange {
            column: "event_date".to_string(),
            start: window_start.to_string(),
            end: window_end.to_string(),
            axis: smelt_backend::PartitionAxis::Calendar,
        };

        match &kind {
            TargetKind::DuckDb => {
                #[cfg(feature = "duckdb")]
                rt.block_on(async {
                    use smelt_backend::Backend;
                    use smelt_backend_duckdb::DuckDbBackend;

                    let backend = DuckDbBackend::new(&db_path, schema)
                        .await
                        .unwrap_or_else(|e| panic!("DuckDB open failed: {e}"));

                    // Seed source table.
                    backend
                        .drop_table_if_exists(schema, "txn_source")
                        .await
                        .unwrap();
                    backend
                        .execute_sql(&create_source_sql(schema))
                        .await
                        .unwrap_or_else(|e| panic!("DuckDB create txn_source failed: {e}"));

                    // First run: CREATE TABLE AS for the day-1 window.
                    backend
                        .drop_table_if_exists(schema, "daily_totals")
                        .await
                        .unwrap();
                    backend
                        .create_table_as(
                            schema,
                            "daily_totals",
                            &window_sql(schema, window_start, window_end),
                        )
                        .await
                        .unwrap_or_else(|e| {
                            panic!("DuckDB create_table_as (first run) failed: {e}")
                        });

                    // Second run (same window) — DELETE+INSERT (idempotency test).
                    backend
                        .delete_partitions(schema, "daily_totals", &partition)
                        .await
                        .unwrap_or_else(|e| panic!("DuckDB delete_partitions failed: {e}"));
                    backend
                        .insert_into_from_query(
                            schema,
                            "daily_totals",
                            &window_sql(schema, window_start, window_end),
                        )
                        .await
                        .unwrap_or_else(|e| panic!("DuckDB insert_into_from_query failed: {e}"));
                });
            }
            TargetKind::Spark => {
                // Only reachable when compiled with --features spark AND SPARK_CONNECT_URL is set.
                #[cfg(not(feature = "spark"))]
                panic!("Spark path should only be reached when --features spark is enabled");
                #[cfg(feature = "spark")]
                {
                    use smelt_backend::Backend;
                    use smelt_backend_spark::SparkBackend;

                    let url = spark_connect_url()
                        .expect("SPARK_CONNECT_URL must be set to reach Spark target");
                    let wh = warehouse
                        .to_str()
                        .expect("warehouse path must be valid UTF-8");

                    rt.block_on(async {
                        let backend =
                            SparkBackend::new(&url, "spark_catalog", schema, Some(wh), true)
                                .await
                                .unwrap_or_else(|e| panic!("Spark connect failed: {e}"));

                        // Seed source table (DROP first for inter-run idempotency).
                        backend
                            .drop_table_if_exists(schema, "txn_source")
                            .await
                            .unwrap();
                        backend
                            .execute_sql(&create_source_sql(schema))
                            .await
                            .unwrap_or_else(|e| panic!("Spark create txn_source failed: {e}"));

                        // First run: CREATE TABLE AS for the day-1 window.
                        backend
                            .drop_table_if_exists(schema, "daily_totals")
                            .await
                            .unwrap();
                        backend
                            .create_table_as(
                                schema,
                                "daily_totals",
                                &window_sql(schema, window_start, window_end),
                            )
                            .await
                            .unwrap_or_else(|e| {
                                panic!("Spark create_table_as (first run) failed: {e}")
                            });

                        // Second run (same window) — DELETE+INSERT.
                        // Requires Delta Lake; if not available the test fails loud
                        // (record as blocked — provisioning issue, not a code fix).
                        backend
                            .delete_partitions(schema, "daily_totals", &partition)
                            .await
                            .unwrap_or_else(|e| panic!("Spark delete_partitions failed: {e}"));
                        backend
                            .insert_into_from_query(
                                schema,
                                "daily_totals",
                                &window_sql(schema, window_start, window_end),
                            )
                            .await
                            .unwrap_or_else(|e| panic!("Spark insert_into_from_query failed: {e}"));
                    });
                }
            }
            TargetKind::BigQuery { dataset } => {
                let _ = dataset;
                #[cfg(not(feature = "bigquery"))]
                panic!("BigQuery path should only be reached when --features bigquery is enabled");
                #[cfg(feature = "bigquery")]
                rt.block_on(async {
                    use smelt_backend::Backend;

                    let backend = common::bq_backend(dataset).await;

                    // Seed source table (DROP first for inter-run idempotency).
                    backend
                        .drop_table_if_exists(schema, "txn_source")
                        .await
                        .unwrap();
                    backend
                        .execute_sql(&create_source_sql(schema))
                        .await
                        .unwrap_or_else(|e| panic!("BigQuery create txn_source failed: {e}"));

                    // First run: CREATE TABLE AS for the day-1 window.
                    backend
                        .drop_table_if_exists(schema, "daily_totals")
                        .await
                        .unwrap();
                    backend
                        .create_table_as(
                            schema,
                            "daily_totals",
                            &window_sql(schema, window_start, window_end),
                        )
                        .await
                        .unwrap_or_else(|e| {
                            panic!("BigQuery create_table_as (first run) failed: {e}")
                        });

                    // Second run (same window) — DELETE+INSERT.
                    backend
                        .delete_partitions(schema, "daily_totals", &partition)
                        .await
                        .unwrap_or_else(|e| panic!("BigQuery delete_partitions failed: {e}"));
                    backend
                        .insert_into_from_query(
                            schema,
                            "daily_totals",
                            &window_sql(schema, window_start, window_end),
                        )
                        .await
                        .unwrap_or_else(|e| panic!("BigQuery insert_into_from_query failed: {e}"));
                });
            }
        }

        let actual = fetch_rows(&kind, &db_path, &warehouse, schema, "daily_totals");
        drop_bq_dataset(&kind);
        assert_table_parity(&actual, &expected_rows(), &format!("{kind:?}"));
    }
}
