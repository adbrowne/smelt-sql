//! Live smoke test: Delta Lake is available on the Spark Connect server.
//!
//! Verifies that `CREATE TABLE … USING DELTA`, `MERGE INTO …`, and a
//! subsequent read all succeed — the minimum bar required to call the Spark
//! backend "Delta-capable" and unlock the parity suite (W7·P1).
//!
//! Gated on `SPARK_CONNECT_URL` (skips silently when unset).  When the
//! variable is set but Delta Lake is absent from the server, the CREATE TABLE
//! call fails and the test panics — making it **red** until `spark-up.sh`
//! provisions Delta.

use smelt_backend::Backend as _;
use smelt_backend_spark::SparkBackend;

const TABLE: &str = "smelt_w7p1_delta_smoke";

#[tokio::test]
async fn delta_table_create_and_merge_smoke() {
    let Some(url) = std::env::var("SPARK_CONNECT_URL").ok() else {
        eprintln!("Skipping delta_table_create_and_merge_smoke — set SPARK_CONNECT_URL to enable");
        return;
    };

    let backend = match SparkBackend::new(&url, "spark_catalog", "default", None, true).await {
        Ok(b) => b,
        Err(e) => {
            eprintln!("Skipping — could not connect to Spark: {}", e);
            return;
        }
    };

    // Clean up any leftovers from a previous run.
    let _ = backend
        .execute_sql(&format!(
            "DROP TABLE IF EXISTS spark_catalog.default.{}",
            TABLE
        ))
        .await;

    // CREATE a Delta table.  If Delta Lake is absent this panics with
    // [DATA_SOURCE_NOT_FOUND] DELTA — the expected RED signal.
    backend
        .execute_sql(&format!(
            "CREATE TABLE spark_catalog.default.{} \
             (id INT, val STRING) USING DELTA",
            TABLE
        ))
        .await
        .unwrap_or_else(|e| {
            panic!(
                "CREATE TABLE USING DELTA failed — Delta Lake is not provisioned on the server. \
                 Run `scripts/spark-up.sh` with the Delta packages to fix this. Error: {}",
                e
            )
        });

    // INSERT a seed row.
    backend
        .execute_sql(&format!(
            "INSERT INTO spark_catalog.default.{} VALUES (1, 'original')",
            TABLE
        ))
        .await
        .expect("INSERT should succeed on Delta table");

    // MERGE — update existing row and insert a new one.
    backend
        .execute_sql(&format!(
            "MERGE INTO spark_catalog.default.{tbl} AS target \
             USING (SELECT 1 AS id, 'updated' AS val UNION ALL SELECT 2, 'new') AS source \
             ON target.id = source.id \
             WHEN MATCHED THEN UPDATE SET val = source.val \
             WHEN NOT MATCHED THEN INSERT (id, val) VALUES (source.id, source.val)",
            tbl = TABLE
        ))
        .await
        .expect("MERGE INTO should succeed on Delta table");

    // Read back and verify both rows are present.
    let batches = backend
        .execute_sql(&format!(
            "SELECT id, val FROM spark_catalog.default.{} ORDER BY id",
            TABLE
        ))
        .await
        .expect("SELECT after MERGE should succeed");

    let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
    assert_eq!(
        total_rows, 2,
        "expected 2 rows after MERGE (updated + inserted), got {}",
        total_rows
    );

    // Clean up.
    let _ = backend
        .execute_sql(&format!(
            "DROP TABLE IF EXISTS spark_catalog.default.{}",
            TABLE
        ))
        .await;
}
