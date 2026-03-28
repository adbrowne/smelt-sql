//! Tests for the Spark backend.
//!
//! - `sql_*` tests: Pure SQL generation (always run, no dependencies)
//! - `integration_*` tests: Require a Spark Connect server (gated behind env var)

use smelt_backend::PartitionSpec;

use crate::sql;

// ─── SQL Generation Tests ──────────────────────────────────────────────────

#[test]
fn sql_qualified_name() {
    assert_eq!(
        sql::qualified_name("spark_catalog", "default", "my_table"),
        "spark_catalog.default.my_table"
    );
    assert_eq!(
        sql::qualified_name("unity_catalog", "analytics", "orders"),
        "unity_catalog.analytics.orders"
    );
}

#[test]
fn sql_drop_table() {
    assert_eq!(
        sql::drop_table("spark_catalog.default.my_table"),
        "DROP TABLE IF EXISTS spark_catalog.default.my_table"
    );
}

#[test]
fn sql_drop_view() {
    assert_eq!(
        sql::drop_view("spark_catalog.default.my_view"),
        "DROP VIEW IF EXISTS spark_catalog.default.my_view"
    );
}

#[test]
fn sql_create_table_as() {
    assert_eq!(
        sql::create_table_as("cat.schema.tbl", "SELECT 1 AS id"),
        "CREATE TABLE cat.schema.tbl AS SELECT 1 AS id"
    );
}

#[test]
fn sql_create_view_as() {
    assert_eq!(
        sql::create_view_as("cat.schema.v", "SELECT * FROM t"),
        "CREATE OR REPLACE VIEW cat.schema.v AS SELECT * FROM t"
    );
}

#[test]
fn sql_create_database() {
    assert_eq!(
        sql::create_database("spark_catalog", "analytics"),
        "CREATE DATABASE IF NOT EXISTS spark_catalog.analytics"
    );
}

#[test]
fn sql_select_preview() {
    assert_eq!(
        sql::select_preview("cat.db.tbl", 10),
        "SELECT * FROM cat.db.tbl LIMIT 10"
    );
    assert_eq!(
        sql::select_preview("cat.db.tbl", 0),
        "SELECT * FROM cat.db.tbl LIMIT 0"
    );
}

#[test]
fn sql_delete_partitions_single_value() {
    let partition = PartitionSpec {
        column: "dt".to_string(),
        values: vec!["2024-01-01".to_string()],
    };
    assert_eq!(
        sql::delete_partitions("cat.db.tbl", &partition),
        "DELETE FROM cat.db.tbl WHERE dt IN ('2024-01-01')"
    );
}

#[test]
fn sql_delete_partitions_multiple_values() {
    let partition = PartitionSpec {
        column: "dt".to_string(),
        values: vec![
            "2024-01-01".to_string(),
            "2024-01-02".to_string(),
            "2024-01-03".to_string(),
        ],
    };
    assert_eq!(
        sql::delete_partitions("cat.db.tbl", &partition),
        "DELETE FROM cat.db.tbl WHERE dt IN ('2024-01-01', '2024-01-02', '2024-01-03')"
    );
}

#[test]
fn sql_delete_partitions_escapes_single_quotes() {
    let partition = PartitionSpec {
        column: "name".to_string(),
        values: vec!["it's".to_string(), "they're".to_string()],
    };
    assert_eq!(
        sql::delete_partitions("cat.db.tbl", &partition),
        "DELETE FROM cat.db.tbl WHERE name IN ('it''s', 'they''re')"
    );
}

#[test]
fn sql_insert_into() {
    assert_eq!(
        sql::insert_into("cat.db.tbl", "SELECT 1 AS id, 'test' AS name"),
        "INSERT INTO cat.db.tbl SELECT 1 AS id, 'test' AS name"
    );
}

#[test]
fn sql_merge_into_single_key() {
    let sql = sql::merge_into(
        "cat.db.users",
        "SELECT 1 AS id, 'Alice' AS name",
        &["id".to_string()],
    );
    assert_eq!(
        sql,
        "MERGE INTO cat.db.users AS target \
         USING (SELECT 1 AS id, 'Alice' AS name) AS source \
         ON target.id = source.id \
         WHEN MATCHED THEN UPDATE SET * \
         WHEN NOT MATCHED THEN INSERT *"
    );
}

#[test]
fn sql_merge_into_composite_key() {
    let sql = sql::merge_into(
        "cat.db.events",
        "SELECT * FROM staging",
        &["user_id".to_string(), "event_date".to_string()],
    );
    assert_eq!(
        sql,
        "MERGE INTO cat.db.events AS target \
         USING (SELECT * FROM staging) AS source \
         ON target.user_id = source.user_id AND target.event_date = source.event_date \
         WHEN MATCHED THEN UPDATE SET * \
         WHEN NOT MATCHED THEN INSERT *"
    );
}

#[test]
fn sql_insert_overwrite() {
    let partition = PartitionSpec {
        column: "dt".to_string(),
        values: vec!["2024-01-01".to_string()],
    };
    assert_eq!(
        sql::insert_overwrite("cat.db.daily", "SELECT * FROM staging", &partition),
        "INSERT OVERWRITE TABLE cat.db.daily PARTITION (dt) SELECT * FROM staging"
    );
}

#[test]
fn sql_truncate() {
    assert_eq!(sql::truncate_sql("SELECT 1"), "SELECT 1");

    let long_sql = "x".repeat(300);
    let truncated = sql::truncate_sql(&long_sql);
    assert!(truncated.len() < 210);
    assert!(truncated.ends_with("..."));
}

// ─── Capabilities ──────────────────────────────────────────────────────────

#[test]
fn capabilities_match_spark_semantics() {
    use smelt_backend::BackendCapabilities;

    let caps = BackendCapabilities::spark();

    // Spark doesn't support QUALIFY — smelt-dialect rewrites it as a subquery
    assert!(!caps.supports_qualify);
    // Spark doesn't support CREATE OR REPLACE TABLE
    assert!(!caps.supports_create_or_replace_table);
    // Spark/Delta supports MERGE
    assert!(caps.supports_merge);
    // Spark supports native INSERT OVERWRITE
    assert!(caps.supports_insert_overwrite);
}

// ─── Integration Tests (require Spark Connect) ─────────────────────────────
//
// These tests need a running Spark Connect server. Set SPARK_CONNECT_URL
// to enable them, e.g.:
//
//   docker run -d -p 15002:15002 apache/spark:latest \
//     /opt/spark/sbin/start-connect-server.sh --jars /opt/spark/jars/*
//
//   SPARK_CONNECT_URL=sc://localhost:15002 cargo test -p smelt-backend-spark
//

#[cfg(test)]
mod integration {
    use super::*;
    use smelt_backend::{Backend, Materialization};

    /// Get the Spark Connect URL from the environment, or skip the test.
    fn spark_connect_url() -> Option<String> {
        std::env::var("SPARK_CONNECT_URL").ok()
    }

    /// Helper to create a SparkBackend for integration tests.
    async fn create_test_backend() -> Option<crate::SparkBackend> {
        let url = spark_connect_url()?;
        match crate::SparkBackend::new(&url, "spark_catalog", "default").await {
            Ok(backend) => Some(backend),
            Err(e) => {
                eprintln!(
                    "Skipping integration test — could not connect to Spark: {}",
                    e
                );
                None
            }
        }
    }

    #[tokio::test]
    async fn integration_dialect_and_capabilities() {
        let Some(backend) = create_test_backend().await else {
            return;
        };
        assert_eq!(backend.dialect(), smelt_backend::SqlDialect::SparkSQL);
        assert!(backend.capabilities().supports_merge);
        assert!(backend.capabilities().supports_insert_overwrite);
    }

    #[tokio::test]
    async fn integration_execute_sql() {
        let Some(backend) = create_test_backend().await else {
            return;
        };
        let batches = backend
            .execute_sql("SELECT 1 AS id, 'hello' AS msg")
            .await
            .unwrap();
        assert!(!batches.is_empty());
        let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total_rows, 1);
    }

    #[tokio::test]
    async fn integration_create_table_and_query() {
        let Some(backend) = create_test_backend().await else {
            return;
        };

        let schema = "default";
        let name = "smelt_test_spark_backend";

        // Clean up from previous runs
        let _ = backend.drop_table_if_exists(schema, name).await;

        // Create table
        backend
            .create_table_as(schema, name, "SELECT 1 AS id, 'test' AS name")
            .await
            .unwrap();

        // Verify it exists
        assert!(backend.table_exists(schema, name).await.unwrap());

        // Row count
        assert_eq!(backend.get_row_count(schema, name).await.unwrap(), 1);

        // Preview
        let preview = backend.get_preview(schema, name, 10).await.unwrap();
        let total_rows: usize = preview.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total_rows, 1);

        // Cleanup
        backend.drop_table_if_exists(schema, name).await.unwrap();
        assert!(!backend.table_exists(schema, name).await.unwrap());
    }

    #[tokio::test]
    async fn integration_create_view() {
        let Some(backend) = create_test_backend().await else {
            return;
        };

        let schema = "default";
        let name = "smelt_test_spark_view";

        let _ = backend.drop_view_if_exists(schema, name).await;

        backend
            .create_view_as(schema, name, "SELECT 42 AS answer")
            .await
            .unwrap();

        let count = backend.get_row_count(schema, name).await.unwrap();
        assert_eq!(count, 1);

        backend.drop_view_if_exists(schema, name).await.unwrap();
    }

    #[tokio::test]
    async fn integration_execute_model() {
        let Some(backend) = create_test_backend().await else {
            return;
        };

        let schema = "default";
        let name = "smelt_test_model";

        let _ = backend.drop_table_if_exists(schema, name).await;

        let result = backend
            .execute_model(
                schema,
                name,
                "SELECT 1 AS id UNION ALL SELECT 2 UNION ALL SELECT 3",
                Materialization::Table,
                true,
            )
            .await
            .unwrap();

        assert_eq!(result.model_name, name);
        assert_eq!(result.row_count, 3);
        assert!(result.preview.is_some());

        backend.drop_table_if_exists(schema, name).await.unwrap();
    }

    #[tokio::test]
    async fn integration_merge_into() {
        // MERGE requires Delta Lake tables. On plain Spark (non-Delta),
        // this test verifies the error is reported correctly.
        let Some(backend) = create_test_backend().await else {
            return;
        };

        let schema = "default";
        let name = "smelt_test_merge";

        let _ = backend.drop_table_if_exists(schema, name).await;

        backend
            .create_table_as(
                schema,
                name,
                "SELECT 1 AS id, 'Alice' AS name, 100 AS score",
            )
            .await
            .unwrap();

        let result = backend
            .merge_into(
                schema,
                name,
                "SELECT 1 AS id, 'Alice' AS name, 200 AS score \
                 UNION ALL SELECT 2 AS id, 'Bob' AS name, 150 AS score",
                &["id".to_string()],
            )
            .await;

        match result {
            Ok(()) => {
                // Delta Lake is available — verify merge worked
                assert_eq!(backend.get_row_count(schema, name).await.unwrap(), 2);
            }
            Err(e) => {
                // Non-Delta warehouse: MERGE is unsupported
                let msg = e.to_string();
                assert!(
                    msg.contains("UNSUPPORTED") || msg.contains("not support"),
                    "Unexpected error: {}",
                    msg
                );
                eprintln!("MERGE not supported (no Delta Lake) — skipping assertion");
            }
        }

        backend.drop_table_if_exists(schema, name).await.unwrap();
    }

    #[tokio::test]
    async fn integration_insert_into() {
        let Some(backend) = create_test_backend().await else {
            return;
        };

        let schema = "default";
        let name = "smelt_test_insert";

        let _ = backend.drop_table_if_exists(schema, name).await;

        backend
            .create_table_as(schema, name, "SELECT 1 AS id")
            .await
            .unwrap();

        backend
            .insert_into_from_query(schema, name, "SELECT 2 AS id")
            .await
            .unwrap();

        assert_eq!(backend.get_row_count(schema, name).await.unwrap(), 2);

        backend.drop_table_if_exists(schema, name).await.unwrap();
    }

    #[tokio::test]
    async fn integration_delete_partitions() {
        // DELETE requires Delta Lake tables. On plain Spark (non-Delta),
        // this test verifies the error is reported correctly.
        let Some(backend) = create_test_backend().await else {
            return;
        };

        let schema = "default";
        let name = "smelt_test_delete_part";

        let _ = backend.drop_table_if_exists(schema, name).await;

        backend
            .create_table_as(
                schema,
                name,
                "SELECT '2024-01-01' AS dt, 10 AS val \
                 UNION ALL SELECT '2024-01-01', 20 \
                 UNION ALL SELECT '2024-01-02', 30",
            )
            .await
            .unwrap();

        assert_eq!(backend.get_row_count(schema, name).await.unwrap(), 3);

        let partition = PartitionSpec {
            column: "dt".to_string(),
            values: vec!["2024-01-01".to_string()],
        };
        let result = backend.delete_partitions(schema, name, &partition).await;

        match result {
            Ok(()) => {
                // Delta Lake is available — verify delete worked
                assert_eq!(backend.get_row_count(schema, name).await.unwrap(), 1);
            }
            Err(e) => {
                // Non-Delta warehouse: DELETE is unsupported
                let msg = e.to_string();
                assert!(
                    msg.contains("UNSUPPORTED") || msg.contains("not support"),
                    "Unexpected error: {}",
                    msg
                );
                eprintln!("DELETE not supported (no Delta Lake) — skipping assertion");
            }
        }

        backend.drop_table_if_exists(schema, name).await.unwrap();
    }
}
