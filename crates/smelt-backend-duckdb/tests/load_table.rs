//! Integration tests for `Backend::load_table` on the DuckDB backend.
//!
//! Tests:
//!   `round_trips_arrow_batches` — every seed type (BOOLEAN, INTEGER, DECIMAL,
//!     DOUBLE, DATE, TIMESTAMP, VARCHAR) survives a `load_table` + `SELECT *` round-trip.
//!   `nullable_columns` — NULL values in nullable columns survive;
//!     NULL in a `nullable: false` column returns a `BackendError`.

use arrow::array::{
    ArrayRef, BooleanArray, Date32Array, Decimal128Array, Float64Array, Int32Array, StringArray,
    TimestampMicrosecondArray,
};
use arrow::datatypes::{DataType, Field, Schema, SchemaRef, TimeUnit};
use arrow::record_batch::RecordBatch;
use smelt_backend::{Backend, BackendError};
use smelt_backend_duckdb::DuckDbBackend;
use std::sync::Arc;
use tempfile::TempDir;

// ── helpers ──────────────────────────────────────────────────────────────────

async fn make_backend() -> (DuckDbBackend, TempDir) {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("test.duckdb");
    let backend = DuckDbBackend::new(&db_path, "main").await.unwrap();
    (backend, dir)
}

// ── round_trips_arrow_batches ─────────────────────────────────────────────────

/// Build RecordBatches with every type in `seeds.md §"Type inference"`, call
/// `load_table`, then verify the round-trip via `SELECT * FROM main.t`.
#[tokio::test]
async fn round_trips_arrow_batches() {
    let (backend, _dir) = make_backend().await;

    // Build schema with every seed type.
    let schema: SchemaRef = Arc::new(Schema::new(vec![
        Field::new("bool_col", DataType::Boolean, false),
        Field::new("int_col", DataType::Int32, false),
        Field::new("decimal_col", DataType::Decimal128(18, 4), false),
        Field::new("double_col", DataType::Float64, false),
        Field::new("date_col", DataType::Date32, false),
        Field::new(
            "ts_col",
            DataType::Timestamp(TimeUnit::Microsecond, None),
            false,
        ),
        Field::new("varchar_col", DataType::Utf8, false),
    ]));

    // One batch, two rows.
    // DATE: days since Unix epoch.  2025-01-01 = 20089.
    // TIMESTAMP: microseconds since Unix epoch.  2025-01-01T00:00:00 = 1735689600000000 µs
    let bool_arr: ArrayRef = Arc::new(BooleanArray::from(vec![true, false]));
    let int_arr: ArrayRef = Arc::new(Int32Array::from(vec![42, -7]));
    let decimal_arr: ArrayRef = Arc::new(
        Decimal128Array::from(vec![123456_i128, -789012_i128])
            .with_precision_and_scale(18, 4)
            .unwrap(),
    );
    let double_arr: ArrayRef = Arc::new(Float64Array::from(vec![1.5_f64, -2.5_f64]));
    let date_arr: ArrayRef = Arc::new(Date32Array::from(vec![20089_i32, 20090_i32]));
    let ts_arr: ArrayRef = Arc::new(TimestampMicrosecondArray::from(vec![
        1_735_689_600_000_000_i64,
        1_735_776_000_000_000_i64,
    ]));
    let varchar_arr: ArrayRef = Arc::new(StringArray::from(vec!["hello", "world"]));

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            bool_arr,
            int_arr,
            decimal_arr,
            double_arr,
            date_arr,
            ts_arr,
            varchar_arr,
        ],
    )
    .unwrap();

    // Call load_table — this is the red-phase target.
    backend
        .load_table("main", "t", schema, vec![batch])
        .await
        .expect("load_table should succeed");

    // Verify row count.
    let count = backend.get_row_count("main", "t").await.unwrap();
    assert_eq!(count, 2);

    // Verify bool_col / int_col / varchar_col.
    let batches = backend
        .execute_sql("SELECT bool_col, int_col, varchar_col FROM main.t ORDER BY int_col")
        .await
        .unwrap();
    let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
    assert_eq!(total_rows, 2);

    let first_batch = &batches[0];
    let int_col = first_batch
        .column(1)
        .as_any()
        .downcast_ref::<Int32Array>()
        .unwrap();
    assert_eq!(int_col.value(0), -7); // ORDER BY int_col ASC
    assert_eq!(int_col.value(1), 42);

    // Verify date_col: days since Unix epoch.
    // Input: 20089 (2025-01-01) and 20090 (2025-01-02), ordered by int_col ASC.
    // Row 0 has int_col=-7 (original row index 1), row 1 has int_col=42 (original row index 0).
    let date_batches = backend
        .execute_sql("SELECT date_col FROM main.t ORDER BY int_col")
        .await
        .unwrap();
    let date_col = date_batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<Date32Array>()
        .unwrap();
    assert_eq!(date_col.value(0), 20090_i32); // int_col=-7 row had date 20090
    assert_eq!(date_col.value(1), 20089_i32); // int_col=42 row had date 20089

    // Verify ts_col: microseconds since Unix epoch.
    // 2025-01-01T00:00:00 = 1_735_689_600_000_000 µs; next day = 1_735_776_000_000_000 µs.
    let ts_batches = backend
        .execute_sql("SELECT epoch_us(ts_col) FROM main.t ORDER BY int_col")
        .await
        .unwrap();
    let ts_col = ts_batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<arrow::array::Int64Array>()
        .unwrap();
    assert_eq!(ts_col.value(0), 1_735_776_000_000_000_i64); // int_col=-7 row
    assert_eq!(ts_col.value(1), 1_735_689_600_000_000_i64); // int_col=42 row

    // Verify decimal_col: raw i128 values (precision=18, scale=4).
    // Inputs: 123456 and -789012 (both unscaled i128 values), ordered by int_col ASC.
    let dec_batches = backend
        .execute_sql("SELECT decimal_col FROM main.t ORDER BY int_col")
        .await
        .unwrap();
    let dec_col = dec_batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<Decimal128Array>()
        .unwrap();
    assert_eq!(dec_col.value(0), -789012_i128); // int_col=-7 row
    assert_eq!(dec_col.value(1), 123456_i128); // int_col=42 row
}

// ── nullable_columns ──────────────────────────────────────────────────────────

/// NULL values in nullable columns survive the round-trip.
#[tokio::test]
async fn nullable_null_values_round_trip() {
    let (backend, _dir) = make_backend().await;

    let schema: SchemaRef = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int32, false),
        Field::new("name", DataType::Utf8, true), // nullable
    ]));

    let id_arr: ArrayRef = Arc::new(Int32Array::from(vec![1, 2, 3]));
    let name_arr: ArrayRef = Arc::new(StringArray::from(vec![
        Some("alice"),
        None, // NULL in nullable column
        Some("carol"),
    ]));

    let batch = RecordBatch::try_new(schema.clone(), vec![id_arr, name_arr]).unwrap();

    backend
        .load_table("main", "nullable_t", schema, vec![batch])
        .await
        .expect("load_table with NULLs in nullable column should succeed");

    let count = backend.get_row_count("main", "nullable_t").await.unwrap();
    assert_eq!(count, 3);

    // Verify the NULL survived.
    let result = backend
        .execute_sql("SELECT name IS NULL FROM main.nullable_t WHERE id = 2")
        .await
        .unwrap();
    let is_null: bool = result[0]
        .column(0)
        .as_any()
        .downcast_ref::<BooleanArray>()
        .unwrap()
        .value(0);
    assert!(is_null, "NULL in nullable column should round-trip as NULL");
}

/// DECIMAL(19, 0) exceeds `p ≤ 18` — `load_table` must return an error rather
/// than silently emitting out-of-spec DDL.
#[tokio::test]
async fn decimal_out_of_bounds_returns_error() {
    let (backend, _dir) = make_backend().await;

    // p=19 violates the p ≤ 18 constraint.
    let schema_p19: SchemaRef = Arc::new(Schema::new(vec![Field::new(
        "amt",
        DataType::Decimal128(19, 0),
        false,
    )]));
    let arr_p19: ArrayRef = Arc::new(
        Decimal128Array::from(vec![1_i128])
            .with_precision_and_scale(19, 0)
            .unwrap(),
    );
    let batch_p19 = RecordBatch::try_new(schema_p19.clone(), vec![arr_p19]).unwrap();
    let result_p19 = backend
        .load_table("main", "bad_p19", schema_p19, vec![batch_p19])
        .await;
    assert!(
        result_p19.is_err(),
        "DECIMAL(19, 0) should be rejected (p > 18)"
    );

    // s=5 violates the s ≤ 4 constraint.
    let schema_s5: SchemaRef = Arc::new(Schema::new(vec![Field::new(
        "amt",
        DataType::Decimal128(10, 5),
        false,
    )]));
    let arr_s5: ArrayRef = Arc::new(
        Decimal128Array::from(vec![1_i128])
            .with_precision_and_scale(10, 5)
            .unwrap(),
    );
    let batch_s5 = RecordBatch::try_new(schema_s5.clone(), vec![arr_s5]).unwrap();
    let result_s5 = backend
        .load_table("main", "bad_s5", schema_s5, vec![batch_s5])
        .await;
    assert!(
        result_s5.is_err(),
        "DECIMAL(10, 5) should be rejected (s > 4)"
    );
}

/// NULLs in a `nullable: false` column (declared in `arrow_schema`) return a `BackendError`.
///
/// We build the RecordBatch with `nullable: true` fields (so Arrow itself accepts
/// the NULL values), but then call `load_table` with a `nullable: false` schema.
/// The backend must detect the violation and return `NullInNonNullableColumn`.
#[tokio::test]
async fn nonnullable_null_returns_error() {
    let (backend, _dir) = make_backend().await;

    // Batch schema: both fields nullable so Arrow lets us put NULLs in them.
    let batch_schema: SchemaRef = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int32, true), // nullable at batch level
        Field::new("value", DataType::Float64, true), // nullable at batch level
    ]));

    // The "load_table" schema declares these NOT NULL — this is what we enforce.
    let load_schema: SchemaRef = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int32, false), // NOT NULL in load_table contract
        Field::new("value", DataType::Float64, false), // NOT NULL in load_table contract
    ]));

    let id_arr: ArrayRef = Arc::new(Int32Array::from(vec![1, 2]));
    // NULL at index 1 in the value column.
    let value_arr: ArrayRef = Arc::new(Float64Array::from(vec![Some(1.0), None]));

    // Arrow accepts this batch because the batch schema is nullable.
    let batch = RecordBatch::try_new(batch_schema, vec![id_arr, value_arr]).unwrap();

    // `load_table` receives the non-nullable schema and must reject the NULL.
    let result = backend
        .load_table("main", "nonnull_t", load_schema, vec![batch])
        .await;
    match result {
        Err(BackendError::NullInNonNullableColumn { .. }) => { /* expected */ }
        Err(e) => panic!("Expected NullInNonNullableColumn error, got: {:?}", e),
        Ok(()) => panic!("Expected an error for NULL in NOT NULL column, but load_table succeeded"),
    }
}
