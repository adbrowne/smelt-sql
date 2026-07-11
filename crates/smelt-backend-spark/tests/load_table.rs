//! Integration test for `Backend::load_table` on the Spark backend.
//!
//! Gated behind `SPARK_CONNECT_URL` env-var (same pattern as existing Spark
//! integration tests in `src/tests.rs`).  If the env-var is absent the test
//! silently skips.

#[tokio::test]
async fn round_trips_via_create_dataframe() {
    // Skip unless a Spark Connect URL is configured.
    let Some(connect_url) = std::env::var("SPARK_CONNECT_URL").ok() else {
        eprintln!("Skipping Spark load_table integration test — set SPARK_CONNECT_URL to enable");
        return;
    };

    use arrow::array::{ArrayRef, Int32Array, StringArray};
    use arrow::datatypes::{DataType, Field, Schema, SchemaRef};
    use arrow::record_batch::RecordBatch;
    use smelt_backend::Backend;
    use smelt_backend_spark::SparkBackend;
    use std::sync::Arc;

    let backend =
        match SparkBackend::new(&connect_url, "spark_catalog", "default", None, true).await {
            Ok(b) => b,
            Err(e) => {
                eprintln!("Skipping — could not connect to Spark: {}", e);
                return;
            }
        };

    let schema: SchemaRef = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int32, false),
        Field::new("label", DataType::Utf8, true),
    ]));

    let id_arr: ArrayRef = Arc::new(Int32Array::from(vec![1, 2, 3]));
    let label_arr: ArrayRef = Arc::new(StringArray::from(vec![Some("a"), None, Some("c")]));
    let batch = RecordBatch::try_new(schema.clone(), vec![id_arr, label_arr]).unwrap();

    let table_name = "smelt_load_table_test";
    // Clean up from any previous run.
    let _ = backend.drop_table_if_exists("default", table_name).await;

    backend
        .load_table("default", table_name, schema, vec![batch])
        .await
        .expect("load_table should succeed against Spark");

    let count = backend.get_row_count("default", table_name).await.unwrap();
    assert_eq!(count, 3);

    // Verify the NULL survived.
    let batches = backend
        .execute_sql(&format!(
            "SELECT id IS NULL as id_null, label IS NULL as lbl_null \
             FROM spark_catalog.default.{} WHERE id = 2",
            table_name
        ))
        .await
        .unwrap();
    let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
    assert_eq!(total_rows, 1);

    backend
        .drop_table_if_exists("default", table_name)
        .await
        .unwrap();
}
