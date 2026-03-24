#![cfg(feature = "duckdb")]
//! Integration test for incremental materialization

use smelt_backend::{Backend, PartitionSpec};
use smelt_backend_duckdb::DuckDbBackend;
use tempfile::TempDir;

/// Seed the test database with source data
async fn seed_database(backend: &DuckDbBackend) -> anyhow::Result<()> {
    // Create raw schema
    backend
        .execute_sql("CREATE SCHEMA IF NOT EXISTS raw")
        .await?;

    // Create users table
    backend
        .execute_sql(
            r#"
            CREATE TABLE IF NOT EXISTS raw.users AS
            SELECT * FROM (VALUES
                (1, 'Alice', 'alice@example.com', '2024-01-01'::DATE),
                (2, 'Bob', 'bob@example.com', '2024-01-02'::DATE),
                (3, 'Charlie', 'charlie@example.com', '2024-01-03'::DATE)
            ) AS t(id, name, email, created_at)
        "#,
        )
        .await?;

    // Create events table
    backend
        .execute_sql(
            r#"
            CREATE TABLE IF NOT EXISTS raw.events AS
            SELECT * FROM (VALUES
                (1, 1, 'login', '2024-12-01 10:00:00'::TIMESTAMP),
                (2, 1, 'view_page', '2024-12-01 10:05:00'::TIMESTAMP),
                (3, 2, 'login', '2024-12-01 11:00:00'::TIMESTAMP),
                (4, 2, 'purchase', '2024-12-01 11:30:00'::TIMESTAMP),
                (5, 3, 'login', '2024-12-02 09:00:00'::TIMESTAMP)
            ) AS t(id, user_id, event_type, event_timestamp)
        "#,
        )
        .await?;

    // Create transactions table with dates for incremental testing
    backend
        .execute_sql(
            r#"
            CREATE TABLE IF NOT EXISTS raw.transactions AS
            SELECT * FROM (VALUES
                (1, 1, 100.00, '2024-12-25 10:00:00'::TIMESTAMP),
                (2, 2, 200.00, '2024-12-25 14:00:00'::TIMESTAMP),
                (3, 1, 50.00, '2024-12-26 09:00:00'::TIMESTAMP),
                (4, 3, 300.00, '2024-12-26 16:00:00'::TIMESTAMP),
                (5, 2, 75.00, '2024-12-27 11:00:00'::TIMESTAMP)
            ) AS t(id, user_id, amount, transaction_timestamp)
        "#,
        )
        .await?;

    Ok(())
}

#[tokio::test]
async fn test_incremental_delete_and_insert() -> anyhow::Result<()> {
    // Create temp database
    let temp_dir = TempDir::new()?;
    let db_path = temp_dir.path().join("test.duckdb");

    let backend = DuckDbBackend::new(&db_path, "main").await?;

    // Seed source data
    seed_database(&backend).await?;

    // Verify source data exists
    let count = backend.get_row_count("raw", "transactions").await?;
    assert_eq!(count, 5, "Expected 5 transactions");

    // Create the daily_revenue table (simulating full refresh)
    backend
        .execute_sql(
            r#"
            CREATE TABLE IF NOT EXISTS main.daily_revenue AS
            SELECT
                transaction_timestamp::DATE as revenue_date,
                user_id,
                SUM(amount) as total_revenue,
                COUNT(*) as transaction_count
            FROM raw.transactions
            GROUP BY 1, 2
        "#,
        )
        .await?;

    // Verify full table
    let count = backend.get_row_count("main", "daily_revenue").await?;
    assert!(count > 0, "Expected rows in daily_revenue");

    // Test delete_partitions
    let partition = PartitionSpec {
        column: "revenue_date".to_string(),
        values: vec!["2024-12-25".to_string()],
    };

    backend
        .delete_partitions("main", "daily_revenue", &partition)
        .await?;

    // Verify rows were deleted
    let result = backend
        .execute_sql("SELECT COUNT(*) FROM main.daily_revenue WHERE revenue_date = '2024-12-25'")
        .await?;
    let count: i64 = result[0]
        .column(0)
        .as_any()
        .downcast_ref::<arrow::array::Int64Array>()
        .unwrap()
        .value(0);
    assert_eq!(count, 0, "Expected 0 rows for 2024-12-25 after delete");

    // Test insert_into_from_query (simulating incremental insert)
    backend
        .insert_into_from_query(
            "main",
            "daily_revenue",
            r#"
            SELECT
                transaction_timestamp::DATE as revenue_date,
                user_id,
                SUM(amount) as total_revenue,
                COUNT(*) as transaction_count
            FROM raw.transactions
            WHERE transaction_timestamp >= '2024-12-25' AND transaction_timestamp < '2024-12-26'
            GROUP BY 1, 2
        "#,
        )
        .await?;

    // Verify rows were re-inserted
    let result = backend
        .execute_sql("SELECT COUNT(*) FROM main.daily_revenue WHERE revenue_date = '2024-12-25'")
        .await?;
    let count: i64 = result[0]
        .column(0)
        .as_any()
        .downcast_ref::<arrow::array::Int64Array>()
        .unwrap()
        .value(0);
    assert!(count > 0, "Expected rows for 2024-12-25 after insert");

    Ok(())
}

#[tokio::test]
async fn test_inject_time_filter() -> anyhow::Result<()> {
    use smelt_cli::{inject_time_filter, TimeRange};

    let sql = "SELECT * FROM smelt.ref('transactions')";
    let range = TimeRange {
        start: "2024-12-25".into(),
        end: "2024-12-26".into(),
    };

    let result = inject_time_filter(sql, "transaction_timestamp", &range)?;

    assert!(result.contains("WHERE transaction_timestamp >= '2024-12-25'"));
    assert!(result.contains("AND transaction_timestamp < '2024-12-26'"));

    Ok(())
}

#[tokio::test]
async fn test_inject_time_filter_with_existing_where() -> anyhow::Result<()> {
    use smelt_cli::{inject_time_filter, TimeRange};

    let sql = "SELECT * FROM smelt.ref('transactions') WHERE user_id = 1";
    let range = TimeRange {
        start: "2024-12-25".into(),
        end: "2024-12-26".into(),
    };

    let result = inject_time_filter(sql, "transaction_timestamp", &range)?;

    // Should keep original WHERE
    assert!(result.contains("WHERE user_id = 1"));
    // Should add AND with filter
    assert!(result.contains(
        "AND (transaction_timestamp >= '2024-12-25' AND transaction_timestamp < '2024-12-26')"
    ));

    Ok(())
}

#[tokio::test]
async fn test_incremental_merge() -> anyhow::Result<()> {
    let temp_dir = TempDir::new()?;
    let db_path = temp_dir.path().join("test.duckdb");

    let backend = DuckDbBackend::new(&db_path, "main").await?;
    seed_database(&backend).await?;

    // Create initial daily_revenue table with unique_key = (revenue_date, user_id)
    backend
        .execute_sql(
            r#"
            CREATE TABLE IF NOT EXISTS main.daily_revenue_merge AS
            SELECT
                transaction_timestamp::DATE as revenue_date,
                user_id,
                SUM(amount) as total_revenue,
                COUNT(*) as transaction_count
            FROM raw.transactions
            GROUP BY 1, 2
        "#,
        )
        .await?;

    let initial_count = backend.get_row_count("main", "daily_revenue_merge").await?;
    assert!(initial_count > 0);

    // MERGE with updated data for existing keys and a new key
    backend
        .merge_into(
            "main",
            "daily_revenue_merge",
            r#"
            SELECT
                transaction_timestamp::DATE as revenue_date,
                user_id,
                SUM(amount) * 2 as total_revenue,
                COUNT(*) as transaction_count
            FROM raw.transactions
            WHERE transaction_timestamp >= '2024-12-25' AND transaction_timestamp < '2024-12-26'
            GROUP BY 1, 2
        "#,
            &["revenue_date".to_string(), "user_id".to_string()],
        )
        .await?;

    // Row count should stay the same (merge updates, doesn't add duplicates)
    let after_count = backend.get_row_count("main", "daily_revenue_merge").await?;
    assert_eq!(
        after_count, initial_count,
        "MERGE should update existing rows, not add new ones"
    );

    Ok(())
}

#[tokio::test]
async fn test_incremental_append() -> anyhow::Result<()> {
    let temp_dir = TempDir::new()?;
    let db_path = temp_dir.path().join("test.duckdb");

    let backend = DuckDbBackend::new(&db_path, "main").await?;
    seed_database(&backend).await?;

    // Create initial table
    backend
        .execute_sql(
            r#"
            CREATE TABLE IF NOT EXISTS main.event_log AS
            SELECT
                transaction_timestamp::DATE as event_date,
                user_id,
                amount
            FROM raw.transactions
            WHERE transaction_timestamp < '2024-12-26'
        "#,
        )
        .await?;

    let initial_count = backend.get_row_count("main", "event_log").await?;

    // APPEND: just insert, no delete
    backend
        .insert_into_from_query(
            "main",
            "event_log",
            r#"
            SELECT
                transaction_timestamp::DATE as event_date,
                user_id,
                amount
            FROM raw.transactions
            WHERE transaction_timestamp >= '2024-12-26' AND transaction_timestamp < '2024-12-28'
        "#,
        )
        .await?;

    let after_count = backend.get_row_count("main", "event_log").await?;
    assert!(
        after_count > initial_count,
        "APPEND should add rows (before: {}, after: {})",
        initial_count,
        after_count
    );

    Ok(())
}
