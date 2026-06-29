//! W5·P4 — MERGE (cumulative upsert) parity across backends.
//!
//! Exercises `Backend::merge_into` on both DuckDB and Spark: create an initial
//! table, merge a second batch that updates existing rows and inserts new rows,
//! then assert the final state matches on each backend.
//!
//! MERGE on Spark requires a Delta-format table.  With a standard Spark session
//! (no Delta runtime), `merge_into` fails; the test surfaces that as a block
//! for the human, not a code fix.
//!
//! With `SPARK_CONNECT_URL` unset: DuckDB only (Spark path skips green).
//! With `SPARK_CONNECT_URL` set AND `--features spark`: also covers Spark.

mod common;
#[cfg(feature = "spark")]
use common::spark_connect_url;
use common::{assert_table_parity, fetch_rows, targets_to_run, TargetKind};
use tempfile::TempDir;

const SPARK_SCHEMA: &str = "smelt_merge_p4";

/// The SELECT that produces the initial batch: two users with scores.
///
/// Uses UNION ALL for cross-dialect compatibility (VALUES is DuckDB-idiomatic;
/// UNION ALL works on both DuckDB and Spark SQL).
fn initial_batch_select() -> &'static str {
    "SELECT 'A' AS user_id, CAST(100 AS BIGINT) AS total_score \
     UNION ALL SELECT 'B', CAST(200 AS BIGINT)"
}

/// The second batch to MERGE: update user A, insert new user C.
///
/// Matched rows (A) get their score updated; unmatched rows (C) are inserted.
fn second_batch_source_sql() -> &'static str {
    "SELECT 'A' AS user_id, CAST(300 AS BIGINT) AS total_score \
     UNION ALL SELECT 'C', CAST(50 AS BIGINT)"
}

/// Expected final state after the MERGE:
/// - A: updated from 100 → 300
/// - B: unchanged at 200 (not in the second batch)
/// - C: inserted at 50 (new)
fn expected_rows() -> Vec<Vec<String>> {
    let mut rows = vec![
        vec!["A".to_string(), "300".to_string()],
        vec!["B".to_string(), "200".to_string()],
        vec!["C".to_string(), "50".to_string()],
    ];
    rows.sort();
    rows
}

/// MERGE (upsert) matches across DuckDB and Spark:
/// - initial table is created with two rows
/// - a second batch updates one existing row and inserts one new row
/// - assert the final state on each backend matches expected rows
///
/// **Red on Spark** if:
/// - `merge_into` is rejected (non-Delta table, unsupported MERGE shape)
/// - final state diverges from DuckDB (wrong update semantics, row duplication)
///
/// If the server lacks a Delta runtime the test fails loud — record as blocked
/// (provisioning issue, not a code fix).
///
/// **DuckDB always green; Spark skips when `SPARK_CONNECT_URL` is unset.**
#[cfg(feature = "duckdb")]
#[test]
fn cumulative_merge_matches_across_backends() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.duckdb");
    let warehouse = tmp.path().join("warehouse");
    std::fs::create_dir_all(&warehouse).unwrap();
    let rt = tokio::runtime::Runtime::new().unwrap();

    for kind in targets_to_run() {
        let schema = match &kind {
            TargetKind::DuckDb => "main",
            TargetKind::Spark => SPARK_SCHEMA,
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

                    // create_table_as handles DROP + CREATE cleanly.
                    backend
                        .create_table_as(schema, "merge_target", initial_batch_select())
                        .await
                        .unwrap_or_else(|e| panic!("DuckDB create merge_target failed: {e}"));

                    // MERGE second batch: A→300 (update), C→50 (insert).
                    backend
                        .merge_into(
                            schema,
                            "merge_target",
                            second_batch_source_sql(),
                            &["user_id".to_string()],
                        )
                        .await
                        .unwrap_or_else(|e| panic!("DuckDB merge_into failed: {e}"));
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

                        // create_table_as adds USING DELTA (use_delta=true) — required for MERGE.
                        backend
                            .create_table_as(schema, "merge_target", initial_batch_select())
                            .await
                            .unwrap_or_else(|e| panic!("Spark create merge_target failed: {e}"));

                        // MERGE second batch.
                        // Requires Delta Lake; if the runtime is absent the test fails loud —
                        // record as blocked (provisioning issue, not a code fix).
                        backend
                            .merge_into(
                                schema,
                                "merge_target",
                                second_batch_source_sql(),
                                &["user_id".to_string()],
                            )
                            .await
                            .unwrap_or_else(|e| panic!("Spark merge_into failed: {e}"));
                    });
                }
            }
        }

        let actual = fetch_rows(&kind, &db_path, &warehouse, schema, "merge_target");
        assert_table_parity(&actual, &expected_rows(), &format!("{kind:?}"));
    }
}
