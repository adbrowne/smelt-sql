//! Tests for schema evolution: column additions, type changes, and continued
//! incremental execution after schema migration.

use super::*;
use smelt_state::file_store::FileStore;
use smelt_state::schema_tracking::{DeployedColumn, DeployedSchema};

#[tokio::test]
async fn test_schema_evolution_add_column_then_continue_incremental() -> Result<()> {
    let (_dir, backend) = setup_backend().await?;
    seed_transactions(&backend).await?;

    // Phase 1: Create initial table (v1 schema: revenue_date, user_id, total_revenue)
    let v1_sql = r#"
        SELECT
            transaction_timestamp::DATE as revenue_date,
            user_id,
            SUM(amount) as total_revenue
        FROM raw.transactions
        WHERE transaction_timestamp >= '2024-12-25' AND transaction_timestamp < '2024-12-27'
        GROUP BY 1, 2
    "#;

    backend.drop_table_if_exists("main", "evolving_model").await?;
    backend.create_table_as("main", "evolving_model", v1_sql).await?;

    let initial_count = backend.get_row_count("main", "evolving_model").await?;
    assert!(initial_count > 0);

    // Phase 2: Schema evolves — add transaction_count column
    backend
        .execute_sql("ALTER TABLE main.evolving_model ADD COLUMN transaction_count BIGINT")
        .await?;

    // Phase 3: Continue incremental with new schema
    let v2_filtered = r#"
        SELECT
            transaction_timestamp::DATE as revenue_date,
            user_id,
            SUM(amount) as total_revenue,
            COUNT(*) as transaction_count
        FROM raw.transactions
        WHERE transaction_timestamp >= '2024-12-27' AND transaction_timestamp < '2024-12-30'
        GROUP BY 1, 2
    "#;

    backend
        .insert_into_from_query("main", "evolving_model", v2_filtered)
        .await?;

    let final_count = backend.get_row_count("main", "evolving_model").await?;
    assert!(
        final_count > initial_count,
        "Should have more rows after incremental insert"
    );

    // Verify new column is populated for new rows
    let result = backend
        .execute_sql(
            "SELECT COUNT(*) FROM main.evolving_model WHERE transaction_count IS NOT NULL",
        )
        .await?;
    let non_null_count = extract_count(&result[0]);
    assert!(non_null_count > 0, "New rows should have transaction_count");

    Ok(())
}

#[tokio::test]
async fn test_schema_diff_detection() -> Result<()> {
    let (dir, _backend) = setup_backend().await?;
    let file_store = FileStore::new(dir.path());

    // Save initial schema
    let v1 = DeployedSchema {
        model: "test_model".to_string(),
        version: 1,
        deployed_at: chrono::Utc::now(),
        model_hash: "sha256:aaa".to_string(),
        columns: vec![
            DeployedColumn {
                name: "id".to_string(),
                data_type: "INTEGER".to_string(),
                nullable: false,
            },
            DeployedColumn {
                name: "amount".to_string(),
                data_type: "DOUBLE".to_string(),
                nullable: true,
            },
        ],
    };
    file_store.save_schema(&v1)?;

    // Load and verify
    let loaded = file_store.load_schema("test_model")?;
    assert!(loaded.is_some());
    let loaded = loaded.unwrap();
    assert_eq!(loaded.columns.len(), 2);
    assert_eq!(loaded.version, 1);

    // Diff against new schema with added column
    let new_columns = vec![
        DeployedColumn {
            name: "id".to_string(),
            data_type: "INTEGER".to_string(),
            nullable: false,
        },
        DeployedColumn {
            name: "amount".to_string(),
            data_type: "DOUBLE".to_string(),
            nullable: true,
        },
        DeployedColumn {
            name: "category".to_string(),
            data_type: "VARCHAR".to_string(),
            nullable: true,
        },
    ];

    let diff = smelt_state::schema_tracking::diff_schemas(&loaded.columns, &new_columns);
    assert!(!diff.is_empty(), "Should detect the added column");

    Ok(())
}

#[tokio::test]
async fn test_schema_first_deployment_no_diff() -> Result<()> {
    let (dir, _backend) = setup_backend().await?;
    let file_store = FileStore::new(dir.path());

    // No prior schema → first deployment
    let loaded = file_store.load_schema("brand_new_model")?;
    assert!(loaded.is_none(), "Should be None for first deployment");

    Ok(())
}

#[tokio::test]
async fn test_incremental_continues_after_full_refresh_on_type_change() -> Result<()> {
    let (_dir, backend) = setup_backend().await?;
    seed_transactions(&backend).await?;

    // Initial table with INTEGER amounts
    backend
        .execute_sql(
            r#"
            CREATE TABLE main.type_change_model AS
            SELECT
                transaction_timestamp::DATE as revenue_date,
                CAST(SUM(amount) AS INTEGER) as total_revenue
            FROM raw.transactions
            WHERE transaction_timestamp >= '2024-12-25' AND transaction_timestamp < '2024-12-27'
            GROUP BY 1
        "#,
        )
        .await?;

    let initial_count = backend.get_row_count("main", "type_change_model").await?;

    // Type change requires full refresh: DROP + CREATE with new schema
    backend
        .drop_table_if_exists("main", "type_change_model")
        .await?;
    backend
        .execute_sql(
            r#"
            CREATE TABLE main.type_change_model AS
            SELECT
                transaction_timestamp::DATE as revenue_date,
                SUM(amount) as total_revenue
            FROM raw.transactions
            WHERE transaction_timestamp >= '2024-12-25' AND transaction_timestamp < '2024-12-27'
            GROUP BY 1
        "#,
        )
        .await?;

    // Now continue incrementally with new schema
    backend
        .insert_into_from_query(
            "main",
            "type_change_model",
            r#"
            SELECT
                transaction_timestamp::DATE as revenue_date,
                SUM(amount) as total_revenue
            FROM raw.transactions
            WHERE transaction_timestamp >= '2024-12-27' AND transaction_timestamp < '2024-12-30'
            GROUP BY 1
        "#,
        )
        .await?;

    let final_count = backend
        .get_row_count("main", "type_change_model")
        .await?;
    assert!(
        final_count > initial_count,
        "Should have added rows after type-change full refresh + incremental"
    );

    Ok(())
}

fn extract_count(batch: &arrow::array::RecordBatch) -> i64 {
    batch
        .column(0)
        .as_any()
        .downcast_ref::<arrow::array::Int64Array>()
        .unwrap()
        .value(0)
}
