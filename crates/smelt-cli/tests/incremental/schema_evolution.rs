//! Tests for schema evolution: column additions, type changes, and continued
//! incremental execution after schema migration.

use super::*;
use smelt_core::config::StateMode;
use smelt_state::file_store::FileStore;
use smelt_state::schema_tracking::{DeployedColumn, DeployedSchema};

/// A retry policy that never retries — these tests drive
/// `check_and_migrate` directly against a real DuckDB backend rather than
/// through `execute_project`, so there is no `ExecuteRequest`/run reporter to
/// derive a policy from (`docs/plans/20260719-prod-w2-operability.md` Phase
/// 6). `retry_max: 0` keeps behaviour identical to before retry coverage was
/// extended to this call site.
const NO_OP_REPORTER: smelt_runtime::NoOpReporter = smelt_runtime::NoOpReporter;
fn no_retry_policy() -> smelt_runtime::RetryPolicy<'static> {
    smelt_runtime::RetryPolicy {
        retry_max: 0,
        base_backoff_ms: 0,
        run_id: "schema-evolution-test",
        model_name: "schema-evolution-test",
        reporter: &NO_OP_REPORTER,
    }
}

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

    backend
        .drop_table_if_exists("main", "evolving_model")
        .await?;
    backend
        .create_table_as("main", "evolving_model", v1_sql)
        .await?;

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
        .execute_sql("SELECT COUNT(*) FROM main.evolving_model WHERE transaction_count IS NOT NULL")
        .await?;
    let non_null_count = extract_count(&result[0]);
    assert!(non_null_count > 0, "New rows should have transaction_count");

    Ok(())
}

#[tokio::test]
async fn test_schema_diff_detection() -> Result<()> {
    let (dir, _backend) = setup_backend().await?;
    let file_store = FileStore::new(dir.path(), "dev", StateMode::Environments);

    // Save initial schema
    let v1 = DeployedSchema {
        model: "test_model".to_string(),
        version: 1,
        deployed_at: chrono::Utc::now(),
        model_hash: "sha256:aaa".to_string(),
        definition_sql: String::new(),
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
    let file_store = FileStore::new(dir.path(), "dev", StateMode::Environments);

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

    let final_count = backend.get_row_count("main", "type_change_model").await?;
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

/// End-to-end: DuckDB struct field addition via check_and_migrate
#[tokio::test]
async fn test_e2e_struct_field_addition() -> Result<()> {
    let (dir, backend) = setup_backend().await?;
    let file_store = FileStore::new(dir.path(), "dev", StateMode::Environments);

    // Create table with a struct column
    backend
        .execute_sql(
            "CREATE TABLE main.struct_model AS SELECT \
             1 AS id, \
             {'a': 42}::STRUCT(a INTEGER) AS meta",
        )
        .await?;

    // Save v1 schema
    let v1 = DeployedSchema {
        model: "struct_model".to_string(),
        version: 1,
        deployed_at: chrono::Utc::now(),
        model_hash: "sha256:v1".to_string(),
        definition_sql: String::new(),
        columns: vec![
            DeployedColumn {
                name: "id".to_string(),
                data_type: "INTEGER".to_string(),
                nullable: true,
            },
            DeployedColumn {
                name: "meta".to_string(),
                data_type: "STRUCT(a INTEGER)".to_string(),
                nullable: true,
            },
        ],
    };
    file_store.save_schema(&v1)?;

    // Infer v2 with added struct field
    let inferred = vec![
        DeployedColumn {
            name: "id".to_string(),
            data_type: "INTEGER".to_string(),
            nullable: true,
        },
        DeployedColumn {
            name: "meta".to_string(),
            data_type: "STRUCT(a INTEGER, b VARCHAR)".to_string(),
            nullable: true,
        },
    ];

    let result = smelt_cli::migration::check_and_migrate(
        &backend,
        &file_store,
        "struct_model",
        "SELECT 1",
        "main",
        &inferred,
        false,
        false,
        false, // not dry run — execute DDL
        &std::collections::HashMap::new(),
        &std::collections::HashMap::new(),
        None, // DuckDB default
        &no_retry_policy(),
    )
    .await?;

    match result {
        smelt_cli::migration::SchemaEvolutionResult::Migrated { statements, .. } => {
            assert!(!statements.is_empty(), "Should have ALTER statements");
            // Verify the DDL added the struct field
            assert!(
                statements.iter().any(|s| s.contains("meta.b")),
                "Should have ALTER for meta.b, got: {:?}",
                statements
            );
        }
        other => panic!("Expected Migrated, got {:?}", other),
    }

    // Verify the column was actually added in DuckDB
    let result = backend
        .execute_sql("SELECT meta.b FROM main.struct_model LIMIT 1")
        .await;
    assert!(
        result.is_ok(),
        "struct field b should exist after migration"
    );

    // Verify schema was saved
    let saved = file_store.load_schema("struct_model")?;
    assert!(saved.is_some());
    assert_eq!(saved.unwrap().version, 2);

    Ok(())
}

/// End-to-end: DuckDB array element type widening
#[tokio::test]
async fn test_e2e_array_element_widening() -> Result<()> {
    let (dir, backend) = setup_backend().await?;
    let file_store = FileStore::new(dir.path(), "dev", StateMode::Environments);

    // Create table with integer array
    backend
        .execute_sql(
            "CREATE TABLE main.arr_model AS SELECT \
             [1, 2, 3]::INTEGER[] AS scores",
        )
        .await?;

    // Save v1 schema
    let v1 = DeployedSchema {
        model: "arr_model".to_string(),
        version: 1,
        deployed_at: chrono::Utc::now(),
        model_hash: "sha256:v1".to_string(),
        definition_sql: String::new(),
        columns: vec![DeployedColumn {
            name: "scores".to_string(),
            data_type: "INTEGER[]".to_string(),
            nullable: true,
        }],
    };
    file_store.save_schema(&v1)?;

    // Infer v2 with widened element type
    let inferred = vec![DeployedColumn {
        name: "scores".to_string(),
        data_type: "BIGINT[]".to_string(),
        nullable: true,
    }];

    let result = smelt_cli::migration::check_and_migrate(
        &backend,
        &file_store,
        "arr_model",
        "SELECT 1",
        "main",
        &inferred,
        false,
        false,
        false,
        &std::collections::HashMap::new(),
        &std::collections::HashMap::new(),
        None,
        &no_retry_policy(),
    )
    .await?;

    match result {
        smelt_cli::migration::SchemaEvolutionResult::Migrated { statements, .. } => {
            assert!(!statements.is_empty());
            assert!(
                statements.iter().any(|s| s.contains("BIGINT[]")),
                "Should widen to BIGINT[], got: {:?}",
                statements
            );
        }
        other => panic!("Expected Migrated, got {:?}", other),
    }

    Ok(())
}

/// End-to-end: Spark+Parquet unsupported op without --allow-full-refresh → blocked
#[tokio::test]
async fn test_e2e_spark_parquet_blocked_without_flag() -> Result<()> {
    let (dir, backend) = setup_backend().await?;
    let file_store = FileStore::new(dir.path(), "dev", StateMode::Environments);

    // Create a table (using DuckDB as execution backend, but testing with Spark DDL backend)
    backend
        .execute_sql(
            "CREATE TABLE main.spark_model AS SELECT \
             {'a': 42, 'b': 'hello'}::STRUCT(a INTEGER, b VARCHAR) AS meta",
        )
        .await?;

    // Save v1 schema
    let v1 = DeployedSchema {
        model: "spark_model".to_string(),
        version: 1,
        deployed_at: chrono::Utc::now(),
        model_hash: "sha256:v1".to_string(),
        definition_sql: String::new(),
        columns: vec![DeployedColumn {
            name: "meta".to_string(),
            data_type: "STRUCT(a INTEGER, b VARCHAR)".to_string(),
            nullable: true,
        }],
    };
    file_store.save_schema(&v1)?;

    // Infer v2 with nested type widening (unsupported on Parquet)
    let inferred = vec![DeployedColumn {
        name: "meta".to_string(),
        data_type: "STRUCT(a BIGINT, b VARCHAR)".to_string(),
        nullable: true,
    }];

    // Use Spark+Parquet backend (nested type widening is unsupported)
    let ddl_backend = smelt_cli::migration::ddl_backend_for_dialect(
        smelt_backend::SqlDialect::SparkSQL,
        Some(smelt_core::config::TableFormat::Parquet),
        None,
    );

    let result = smelt_cli::migration::check_and_migrate(
        &backend,
        &file_store,
        "spark_model",
        "SELECT 1",
        "main",
        &inferred,
        false,
        false, // allow_full_refresh = false
        true,  // dry_run = true (don't actually execute Spark DDL against DuckDB)
        &std::collections::HashMap::new(),
        &std::collections::HashMap::new(),
        Some(&ddl_backend),
        &no_retry_policy(),
    )
    .await?;

    match result {
        smelt_cli::migration::SchemaEvolutionResult::FullRefreshBlocked { reason } => {
            assert!(
                reason.contains("Parquet") || reason.contains("full refresh"),
                "Should mention Parquet limitation, got: {}",
                reason
            );
        }
        other => panic!("Expected FullRefreshBlocked, got {:?}", other),
    }

    Ok(())
}

/// End-to-end: Spark+Parquet unsupported op WITH --allow-full-refresh → allowed
#[tokio::test]
async fn test_e2e_spark_parquet_allowed_with_flag() -> Result<()> {
    let (dir, backend) = setup_backend().await?;
    let file_store = FileStore::new(dir.path(), "dev", StateMode::Environments);

    backend
        .execute_sql(
            "CREATE TABLE main.spark_model2 AS SELECT \
             {'a': 42, 'b': 'hello'}::STRUCT(a INTEGER, b VARCHAR) AS meta",
        )
        .await?;

    let v1 = DeployedSchema {
        model: "spark_model2".to_string(),
        version: 1,
        deployed_at: chrono::Utc::now(),
        model_hash: "sha256:v1".to_string(),
        definition_sql: String::new(),
        columns: vec![DeployedColumn {
            name: "meta".to_string(),
            data_type: "STRUCT(a INTEGER, b VARCHAR)".to_string(),
            nullable: true,
        }],
    };
    file_store.save_schema(&v1)?;

    let inferred = vec![DeployedColumn {
        name: "meta".to_string(),
        data_type: "STRUCT(a BIGINT, b VARCHAR)".to_string(),
        nullable: true,
    }];

    let ddl_backend = smelt_cli::migration::ddl_backend_for_dialect(
        smelt_backend::SqlDialect::SparkSQL,
        Some(smelt_core::config::TableFormat::Parquet),
        None,
    );

    let result = smelt_cli::migration::check_and_migrate(
        &backend,
        &file_store,
        "spark_model2",
        "SELECT 1",
        "main",
        &inferred,
        false,
        true, // allow_full_refresh = true
        true, // dry_run = true
        &std::collections::HashMap::new(),
        &std::collections::HashMap::new(),
        Some(&ddl_backend),
        &no_retry_policy(),
    )
    .await?;

    // Should be allowed — either FullRefreshRequired or TableRewrite
    match result {
        smelt_cli::migration::SchemaEvolutionResult::FullRefreshRequired { .. } => {
            // Parquet nested widen → FullRefreshRequired (allow_full_refresh converts FullRefreshBlocked)
        }
        other => panic!("Expected FullRefreshRequired, got {:?}", other),
    }

    Ok(())
}

/// End-to-end: complex type string persisted correctly in schema
#[tokio::test]
async fn test_e2e_complex_type_schema_persistence() -> Result<()> {
    let (dir, backend) = setup_backend().await?;
    let file_store = FileStore::new(dir.path(), "dev", StateMode::Environments);

    // Create table with complex types
    backend
        .execute_sql(
            "CREATE TABLE main.complex_model AS SELECT \
             {'a': 42}::STRUCT(a INTEGER) AS meta, \
             [1, 2, 3]::INTEGER[] AS scores, \
             MAP {'x': 1}::MAP(VARCHAR, INTEGER) AS lookup",
        )
        .await?;

    // Save v1 with complex type strings
    let v1 = DeployedSchema {
        model: "complex_model".to_string(),
        version: 1,
        deployed_at: chrono::Utc::now(),
        model_hash: "sha256:v1".to_string(),
        definition_sql: String::new(),
        columns: vec![
            DeployedColumn {
                name: "meta".to_string(),
                data_type: "STRUCT(a INTEGER)".to_string(),
                nullable: true,
            },
            DeployedColumn {
                name: "scores".to_string(),
                data_type: "INTEGER[]".to_string(),
                nullable: true,
            },
            DeployedColumn {
                name: "lookup".to_string(),
                data_type: "MAP(VARCHAR, INTEGER)".to_string(),
                nullable: true,
            },
        ],
    };
    file_store.save_schema(&v1)?;

    // Save again via save_deployed_schema
    smelt_cli::migration::save_deployed_schema(
        &file_store,
        "complex_model",
        "SELECT 1",
        &v1.columns,
        Some(1),
    )?;

    // Load and verify complex types persisted correctly
    let loaded = file_store.load_schema("complex_model")?;
    assert!(loaded.is_some());
    let loaded = loaded.unwrap();
    assert_eq!(loaded.version, 2);
    assert_eq!(loaded.columns[0].data_type, "STRUCT(a INTEGER)");
    assert_eq!(loaded.columns[1].data_type, "INTEGER[]");
    assert_eq!(loaded.columns[2].data_type, "MAP(VARCHAR, INTEGER)");

    Ok(())
}

/// End-to-end: DuckDB nested type widening via struct_pack rewrite
#[tokio::test]
async fn test_e2e_nested_type_widening() -> Result<()> {
    let (dir, backend) = setup_backend().await?;
    let file_store = FileStore::new(dir.path(), "dev", StateMode::Environments);

    // Create table with struct column containing INTEGER field
    backend
        .execute_sql(
            "CREATE TABLE main.widen_model AS SELECT \
             1 AS id, \
             {'a': 42, 'b': 'hello'}::STRUCT(a INTEGER, b VARCHAR) AS meta",
        )
        .await?;

    // Save v1 schema
    let v1 = DeployedSchema {
        model: "widen_model".to_string(),
        version: 1,
        deployed_at: chrono::Utc::now(),
        model_hash: "sha256:v1".to_string(),
        definition_sql: String::new(),
        columns: vec![
            DeployedColumn {
                name: "id".to_string(),
                data_type: "INTEGER".to_string(),
                nullable: true,
            },
            DeployedColumn {
                name: "meta".to_string(),
                data_type: "STRUCT(a INTEGER, b VARCHAR)".to_string(),
                nullable: true,
            },
        ],
    };
    file_store.save_schema(&v1)?;

    // Infer v2 with widened nested type: INTEGER → BIGINT
    let inferred = vec![
        DeployedColumn {
            name: "id".to_string(),
            data_type: "INTEGER".to_string(),
            nullable: true,
        },
        DeployedColumn {
            name: "meta".to_string(),
            data_type: "STRUCT(a BIGINT, b VARCHAR)".to_string(),
            nullable: true,
        },
    ];

    let result = smelt_cli::migration::check_and_migrate(
        &backend,
        &file_store,
        "widen_model",
        "SELECT 1",
        "main",
        &inferred,
        false,
        false,
        false, // not dry run — execute DDL
        &std::collections::HashMap::new(),
        &std::collections::HashMap::new(),
        None, // DuckDB default
        &no_retry_policy(),
    )
    .await?;

    match result {
        smelt_cli::migration::SchemaEvolutionResult::Migrated { statements, .. } => {
            assert!(!statements.is_empty(), "Should have ALTER statements");
            // Should use ALTER COLUMN TYPE with struct_pack or dot-notation
            let all_stmts = statements.join(" ");
            assert!(
                all_stmts.contains("ALTER") && all_stmts.contains("meta"),
                "Should ALTER meta column, got: {:?}",
                statements
            );
        }
        other => panic!("Expected Migrated, got {:?}", other),
    }

    // Verify the type was actually widened in DuckDB
    let result = backend
        .execute_sql("SELECT typeof(meta.a) FROM main.widen_model LIMIT 1")
        .await?;
    let type_str = result[0]
        .column(0)
        .as_any()
        .downcast_ref::<arrow::array::StringArray>()
        .unwrap()
        .value(0);
    assert_eq!(type_str, "BIGINT", "meta.a should be BIGINT after widening");

    Ok(())
}

/// End-to-end: incompatible type change (struct → scalar) triggers full refresh
#[tokio::test]
async fn test_e2e_incompatible_type_triggers_full_refresh() -> Result<()> {
    let (dir, backend) = setup_backend().await?;
    let file_store = FileStore::new(dir.path(), "dev", StateMode::Environments);

    backend
        .execute_sql(
            "CREATE TABLE main.incompat_model AS SELECT \
             {'a': 42}::STRUCT(a INTEGER) AS meta",
        )
        .await?;

    let v1 = DeployedSchema {
        model: "incompat_model".to_string(),
        version: 1,
        deployed_at: chrono::Utc::now(),
        model_hash: "sha256:v1".to_string(),
        definition_sql: String::new(),
        columns: vec![DeployedColumn {
            name: "meta".to_string(),
            data_type: "STRUCT(a INTEGER)".to_string(),
            nullable: true,
        }],
    };
    file_store.save_schema(&v1)?;

    // Infer v2: struct → scalar (incompatible)
    let inferred = vec![DeployedColumn {
        name: "meta".to_string(),
        data_type: "INTEGER".to_string(),
        nullable: true,
    }];

    let result = smelt_cli::migration::check_and_migrate(
        &backend,
        &file_store,
        "incompat_model",
        "SELECT 1",
        "main",
        &inferred,
        false,
        false,
        true, // dry_run
        &std::collections::HashMap::new(),
        &std::collections::HashMap::new(),
        None,
        &no_retry_policy(),
    )
    .await?;

    match result {
        smelt_cli::migration::SchemaEvolutionResult::FullRefreshRequired { reason } => {
            assert!(
                reason.contains("incompatible") || reason.contains("struct"),
                "Should explain the incompatible change, got: {}",
                reason
            );
        }
        other => panic!("Expected FullRefreshRequired, got {:?}", other),
    }

    Ok(())
}

/// End-to-end: map key type change triggers full refresh
#[tokio::test]
async fn test_e2e_map_key_change_triggers_full_refresh() -> Result<()> {
    let (dir, backend) = setup_backend().await?;
    let file_store = FileStore::new(dir.path(), "dev", StateMode::Environments);

    backend
        .execute_sql(
            "CREATE TABLE main.mapkey_model AS SELECT \
             MAP {'x': 1}::MAP(VARCHAR, INTEGER) AS lookup",
        )
        .await?;

    let v1 = DeployedSchema {
        model: "mapkey_model".to_string(),
        version: 1,
        deployed_at: chrono::Utc::now(),
        model_hash: "sha256:v1".to_string(),
        definition_sql: String::new(),
        columns: vec![DeployedColumn {
            name: "lookup".to_string(),
            data_type: "MAP(VARCHAR, INTEGER)".to_string(),
            nullable: true,
        }],
    };
    file_store.save_schema(&v1)?;

    // Infer v2: map key type changed
    let inferred = vec![DeployedColumn {
        name: "lookup".to_string(),
        data_type: "MAP(INTEGER, INTEGER)".to_string(),
        nullable: true,
    }];

    let result = smelt_cli::migration::check_and_migrate(
        &backend,
        &file_store,
        "mapkey_model",
        "SELECT 1",
        "main",
        &inferred,
        false,
        false,
        true, // dry_run
        &std::collections::HashMap::new(),
        &std::collections::HashMap::new(),
        None,
        &no_retry_policy(),
    )
    .await?;

    match result {
        smelt_cli::migration::SchemaEvolutionResult::FullRefreshRequired { reason } => {
            assert!(
                reason.contains("map key") || reason.contains("lookup"),
                "Should explain the map key change, got: {}",
                reason
            );
        }
        other => panic!("Expected FullRefreshRequired, got {:?}", other),
    }

    Ok(())
}

// ===== Phase 11: DuckDB integration tests for complex type schema evolution =====

/// 11a: Struct field removal with allow_column_removal flag
#[tokio::test]
async fn test_e2e_struct_field_removal() -> Result<()> {
    let (dir, backend) = setup_backend().await?;
    let file_store = FileStore::new(dir.path(), "dev", StateMode::Environments);

    backend
        .execute_sql(
            "CREATE TABLE main.struct_rm AS SELECT \
             1 AS id, \
             {'a': 42, 'b': 'hello'}::STRUCT(a INTEGER, b VARCHAR) AS meta",
        )
        .await?;

    let v1 = DeployedSchema {
        model: "struct_rm".to_string(),
        version: 1,
        deployed_at: chrono::Utc::now(),
        model_hash: "sha256:v1".to_string(),
        definition_sql: String::new(),
        columns: vec![
            DeployedColumn {
                name: "id".to_string(),
                data_type: "INTEGER".to_string(),
                nullable: true,
            },
            DeployedColumn {
                name: "meta".to_string(),
                data_type: "STRUCT(a INTEGER, b VARCHAR)".to_string(),
                nullable: true,
            },
        ],
    };
    file_store.save_schema(&v1)?;

    // Infer v2: struct field 'b' removed
    let inferred = vec![
        DeployedColumn {
            name: "id".to_string(),
            data_type: "INTEGER".to_string(),
            nullable: true,
        },
        DeployedColumn {
            name: "meta".to_string(),
            data_type: "STRUCT(a INTEGER)".to_string(),
            nullable: true,
        },
    ];

    // With allow_column_removal=true, struct field removal should succeed
    let result = smelt_cli::migration::check_and_migrate(
        &backend,
        &file_store,
        "struct_rm",
        "SELECT 1",
        "main",
        &inferred,
        true, // allow_column_removal
        false,
        false, // not dry run — execute DDL
        &std::collections::HashMap::new(),
        &std::collections::HashMap::new(),
        None,
        &no_retry_policy(),
    )
    .await?;

    match result {
        smelt_cli::migration::SchemaEvolutionResult::Migrated { statements, .. } => {
            assert!(!statements.is_empty(), "Should have ALTER statements");
            assert!(
                statements.iter().any(|s| s.contains("meta.b")),
                "Should DROP struct field meta.b, got: {:?}",
                statements
            );
        }
        other => panic!("Expected Migrated, got {:?}", other),
    }

    // Verify the field was actually removed — meta should now be STRUCT(a INTEGER)
    let result = backend
        .execute_sql("SELECT typeof(meta) FROM main.struct_rm LIMIT 1")
        .await?;
    let type_str = result[0]
        .column(0)
        .as_any()
        .downcast_ref::<arrow::array::StringArray>()
        .unwrap()
        .value(0);
    assert!(
        !type_str.contains("VARCHAR"),
        "meta should no longer have b VARCHAR, got: {}",
        type_str
    );

    Ok(())
}

/// 11a: Map value type widening → incremental continues
#[tokio::test]
async fn test_e2e_map_value_widening() -> Result<()> {
    let (dir, backend) = setup_backend().await?;
    let file_store = FileStore::new(dir.path(), "dev", StateMode::Environments);

    backend
        .execute_sql(
            "CREATE TABLE main.mapval_model AS SELECT \
             MAP {'x': 1, 'y': 2}::MAP(VARCHAR, INTEGER) AS lookup",
        )
        .await?;

    let v1 = DeployedSchema {
        model: "mapval_model".to_string(),
        version: 1,
        deployed_at: chrono::Utc::now(),
        model_hash: "sha256:v1".to_string(),
        definition_sql: String::new(),
        columns: vec![DeployedColumn {
            name: "lookup".to_string(),
            data_type: "MAP(VARCHAR, INTEGER)".to_string(),
            nullable: true,
        }],
    };
    file_store.save_schema(&v1)?;

    // Infer v2: map value widened to BIGINT
    let inferred = vec![DeployedColumn {
        name: "lookup".to_string(),
        data_type: "MAP(VARCHAR, BIGINT)".to_string(),
        nullable: true,
    }];

    let result = smelt_cli::migration::check_and_migrate(
        &backend,
        &file_store,
        "mapval_model",
        "SELECT 1",
        "main",
        &inferred,
        false,
        false,
        false,
        &std::collections::HashMap::new(),
        &std::collections::HashMap::new(),
        None,
        &no_retry_policy(),
    )
    .await?;

    match result {
        smelt_cli::migration::SchemaEvolutionResult::Migrated { statements, .. } => {
            assert!(!statements.is_empty(), "Should have ALTER statements");
            let all_stmts = statements.join(" ");
            assert!(
                all_stmts.contains("MAP(VARCHAR, BIGINT)") || all_stmts.contains("BIGINT"),
                "Should widen map value type, got: {:?}",
                statements
            );
        }
        other => panic!("Expected Migrated, got {:?}", other),
    }

    Ok(())
}

/// 11a: Array-of-struct field addition → incremental continues
#[tokio::test]
async fn test_e2e_array_of_struct_field_addition() -> Result<()> {
    let (dir, backend) = setup_backend().await?;
    let file_store = FileStore::new(dir.path(), "dev", StateMode::Environments);

    backend
        .execute_sql(
            "CREATE TABLE main.arr_struct_model AS SELECT \
             [{'a': 1}, {'a': 2}]::STRUCT(a INTEGER)[] AS items",
        )
        .await?;

    let v1 = DeployedSchema {
        model: "arr_struct_model".to_string(),
        version: 1,
        deployed_at: chrono::Utc::now(),
        model_hash: "sha256:v1".to_string(),
        definition_sql: String::new(),
        columns: vec![DeployedColumn {
            name: "items".to_string(),
            data_type: "STRUCT(a INTEGER)[]".to_string(),
            nullable: true,
        }],
    };
    file_store.save_schema(&v1)?;

    // Infer v2: struct inside array gets a new field
    let inferred = vec![DeployedColumn {
        name: "items".to_string(),
        data_type: "STRUCT(a INTEGER, b VARCHAR)[]".to_string(),
        nullable: true,
    }];

    let result = smelt_cli::migration::check_and_migrate(
        &backend,
        &file_store,
        "arr_struct_model",
        "SELECT 1",
        "main",
        &inferred,
        false,
        false,
        false,
        &std::collections::HashMap::new(),
        &std::collections::HashMap::new(),
        None,
        &no_retry_policy(),
    )
    .await?;

    match &result {
        smelt_cli::migration::SchemaEvolutionResult::Migrated { statements, .. } => {
            assert!(
                !statements.is_empty(),
                "Should have migration statements for array-of-struct field add"
            );
        }
        other => panic!("Expected Migrated, got {:?}", other),
    }

    // Verify we can query the new struct field
    let check = backend
        .execute_sql("SELECT typeof(items) FROM main.arr_struct_model LIMIT 1")
        .await?;
    let type_str = check[0]
        .column(0)
        .as_any()
        .downcast_ref::<arrow::array::StringArray>()
        .unwrap()
        .value(0);
    assert!(
        type_str.contains("b"),
        "items should now have field b, got type: {}",
        type_str
    );

    Ok(())
}

/// 11a: Nested struct field addition (deep nesting)
#[tokio::test]
async fn test_e2e_nested_struct_field_addition() -> Result<()> {
    let (dir, backend) = setup_backend().await?;
    let file_store = FileStore::new(dir.path(), "dev", StateMode::Environments);

    backend
        .execute_sql(
            "CREATE TABLE main.nested_add_model AS SELECT \
             {'nested': {'x': 10}}::STRUCT(nested STRUCT(x INTEGER)) AS data",
        )
        .await?;

    let v1 = DeployedSchema {
        model: "nested_add_model".to_string(),
        version: 1,
        deployed_at: chrono::Utc::now(),
        model_hash: "sha256:v1".to_string(),
        definition_sql: String::new(),
        columns: vec![DeployedColumn {
            name: "data".to_string(),
            data_type: "STRUCT(nested STRUCT(x INTEGER))".to_string(),
            nullable: true,
        }],
    };
    file_store.save_schema(&v1)?;

    // Infer v2: nested struct gets a new field
    let inferred = vec![DeployedColumn {
        name: "data".to_string(),
        data_type: "STRUCT(nested STRUCT(x INTEGER, y VARCHAR))".to_string(),
        nullable: true,
    }];

    let result = smelt_cli::migration::check_and_migrate(
        &backend,
        &file_store,
        "nested_add_model",
        "SELECT 1",
        "main",
        &inferred,
        false,
        false,
        false,
        &std::collections::HashMap::new(),
        &std::collections::HashMap::new(),
        None,
        &no_retry_policy(),
    )
    .await?;

    match &result {
        smelt_cli::migration::SchemaEvolutionResult::Migrated { statements, .. } => {
            assert!(!statements.is_empty(), "Should have ALTER statements");
            // Should reference the nested path
            let all_stmts = statements.join(" ");
            assert!(
                all_stmts.contains("data") && all_stmts.contains("nested"),
                "Should reference nested path data.nested, got: {:?}",
                statements
            );
        }
        other => panic!("Expected Migrated, got {:?}", other),
    }

    // Verify the nested field was added
    let result = backend
        .execute_sql("SELECT data.nested.y FROM main.nested_add_model LIMIT 1")
        .await;
    assert!(
        result.is_ok(),
        "data.nested.y should exist after nested field addition"
    );

    Ok(())
}

/// 11a: Multiple changes in one migration (add struct field + widen array type)
#[tokio::test]
async fn test_e2e_multiple_changes_one_migration() -> Result<()> {
    let (dir, backend) = setup_backend().await?;
    let file_store = FileStore::new(dir.path(), "dev", StateMode::Environments);

    backend
        .execute_sql(
            "CREATE TABLE main.multi_model AS SELECT \
             1 AS id, \
             {'a': CAST(42 AS INTEGER), 'b': 'hello'}::STRUCT(a INTEGER, b VARCHAR) AS meta, \
             [1, 2, 3]::INTEGER[] AS scores",
        )
        .await?;

    let v1 = DeployedSchema {
        model: "multi_model".to_string(),
        version: 1,
        deployed_at: chrono::Utc::now(),
        model_hash: "sha256:v1".to_string(),
        definition_sql: String::new(),
        columns: vec![
            DeployedColumn {
                name: "id".to_string(),
                data_type: "INTEGER".to_string(),
                nullable: true,
            },
            DeployedColumn {
                name: "meta".to_string(),
                data_type: "STRUCT(a INTEGER, b VARCHAR)".to_string(),
                nullable: true,
            },
            DeployedColumn {
                name: "scores".to_string(),
                data_type: "INTEGER[]".to_string(),
                nullable: true,
            },
        ],
    };
    file_store.save_schema(&v1)?;

    // Infer v2: struct field added + array element widened
    let inferred = vec![
        DeployedColumn {
            name: "id".to_string(),
            data_type: "INTEGER".to_string(),
            nullable: true,
        },
        DeployedColumn {
            name: "meta".to_string(),
            data_type: "STRUCT(a INTEGER, b VARCHAR, c BOOLEAN)".to_string(),
            nullable: true,
        },
        DeployedColumn {
            name: "scores".to_string(),
            data_type: "BIGINT[]".to_string(),
            nullable: true,
        },
    ];

    let result = smelt_cli::migration::check_and_migrate(
        &backend,
        &file_store,
        "multi_model",
        "SELECT 1",
        "main",
        &inferred,
        false,
        false,
        false,
        &std::collections::HashMap::new(),
        &std::collections::HashMap::new(),
        None,
        &no_retry_policy(),
    )
    .await?;

    match &result {
        smelt_cli::migration::SchemaEvolutionResult::Migrated { statements, .. } => {
            assert!(
                statements.len() >= 2,
                "Should have at least 2 ALTER statements (struct + array), got: {:?}",
                statements
            );
        }
        other => panic!("Expected Migrated, got {:?}", other),
    }

    // Verify both changes applied
    let check = backend
        .execute_sql("SELECT meta.c, typeof(scores) FROM main.multi_model LIMIT 1")
        .await?;
    // meta.c should exist (NULL for old rows)
    assert_eq!(check[0].num_columns(), 2, "Should have both result columns");

    Ok(())
}

/// 11b: struct_pack rewrite produces correct data — insert v1 rows, migrate, verify
#[tokio::test]
async fn test_e2e_struct_pack_data_correctness() -> Result<()> {
    let (dir, backend) = setup_backend().await?;
    let file_store = FileStore::new(dir.path(), "dev", StateMode::Environments);

    // Create table with v1 schema and insert multiple rows
    backend
        .execute_sql(
            "CREATE TABLE main.data_model AS SELECT * FROM (VALUES \
             (1, {'a': CAST(10 AS INTEGER), 'b': 'foo'}::STRUCT(a INTEGER, b VARCHAR)), \
             (2, {'a': CAST(20 AS INTEGER), 'b': 'bar'}::STRUCT(a INTEGER, b VARCHAR)), \
             (3, {'a': CAST(30 AS INTEGER), 'b': 'baz'}::STRUCT(a INTEGER, b VARCHAR))  \
             ) AS t(id, meta)",
        )
        .await?;

    let v1 = DeployedSchema {
        model: "data_model".to_string(),
        version: 1,
        deployed_at: chrono::Utc::now(),
        model_hash: "sha256:v1".to_string(),
        definition_sql: String::new(),
        columns: vec![
            DeployedColumn {
                name: "id".to_string(),
                data_type: "INTEGER".to_string(),
                nullable: true,
            },
            DeployedColumn {
                name: "meta".to_string(),
                data_type: "STRUCT(a INTEGER, b VARCHAR)".to_string(),
                nullable: true,
            },
        ],
    };
    file_store.save_schema(&v1)?;

    // Infer v2: widen meta.a from INTEGER to BIGINT, add field c
    let inferred = vec![
        DeployedColumn {
            name: "id".to_string(),
            data_type: "INTEGER".to_string(),
            nullable: true,
        },
        DeployedColumn {
            name: "meta".to_string(),
            data_type: "STRUCT(a BIGINT, b VARCHAR, c BOOLEAN)".to_string(),
            nullable: true,
        },
    ];

    let result = smelt_cli::migration::check_and_migrate(
        &backend,
        &file_store,
        "data_model",
        "SELECT 1",
        "main",
        &inferred,
        false,
        false,
        false,
        &std::collections::HashMap::new(),
        &std::collections::HashMap::new(),
        None,
        &no_retry_policy(),
    )
    .await?;

    match &result {
        smelt_cli::migration::SchemaEvolutionResult::Migrated { .. } => {}
        other => panic!("Expected Migrated, got {:?}", other),
    }

    // Verify old rows have correct widened values
    let check = backend
        .execute_sql("SELECT id, meta.a, meta.b, meta.c FROM main.data_model ORDER BY id")
        .await?;

    // Check row count
    let total_rows: usize = check.iter().map(|b| b.num_rows()).sum();
    assert_eq!(total_rows, 3, "Should have 3 original rows");

    // Verify types are correct
    let type_check = backend
        .execute_sql("SELECT typeof(meta.a) FROM main.data_model LIMIT 1")
        .await?;
    let a_type = type_check[0]
        .column(0)
        .as_any()
        .downcast_ref::<arrow::array::StringArray>()
        .unwrap()
        .value(0);
    assert_eq!(a_type, "BIGINT", "meta.a should be BIGINT after widening");

    // Verify old rows have NULL for new field c
    let null_check = backend
        .execute_sql("SELECT COUNT(*) FROM main.data_model WHERE meta.c IS NULL")
        .await?;
    let null_count = extract_count(&null_check[0]);
    assert_eq!(null_count, 3, "All old rows should have NULL for meta.c");

    // Verify old data values preserved
    let value_check = backend
        .execute_sql("SELECT meta.a, meta.b FROM main.data_model WHERE id = 1")
        .await?;
    let a_val = value_check[0]
        .column(0)
        .as_any()
        .downcast_ref::<arrow::array::Int64Array>()
        .unwrap()
        .value(0);
    assert_eq!(a_val, 10, "meta.a should still be 10 after widening");

    let b_val = value_check[0]
        .column(1)
        .as_any()
        .downcast_ref::<arrow::array::StringArray>()
        .unwrap()
        .value(0);
    assert_eq!(b_val, "foo", "meta.b should still be 'foo'");

    // Insert new row with v2 schema
    backend
        .execute_sql(
            "INSERT INTO main.data_model VALUES \
             (4, {'a': CAST(40 AS BIGINT), 'b': 'qux', 'c': true})",
        )
        .await?;

    // Verify both old and new rows queryable
    let final_check = backend
        .execute_sql("SELECT COUNT(*) FROM main.data_model")
        .await?;
    let final_count = extract_count(&final_check[0]);
    assert_eq!(final_count, 4, "Should have 4 total rows");

    // New row should have non-null c
    let new_row_check = backend
        .execute_sql("SELECT meta.c FROM main.data_model WHERE id = 4")
        .await?;
    let c_val = new_row_check[0]
        .column(0)
        .as_any()
        .downcast_ref::<arrow::array::BooleanArray>()
        .unwrap()
        .value(0);
    assert!(c_val, "New row meta.c should be true");

    Ok(())
}

/// 11c: Deeply nested struct — inner struct gets field added AND type widened
#[tokio::test]
async fn test_e2e_deeply_nested_struct_widen_and_add() -> Result<()> {
    let (dir, backend) = setup_backend().await?;
    let file_store = FileStore::new(dir.path(), "dev", StateMode::Environments);

    backend
        .execute_sql(
            "CREATE TABLE main.deep_model AS SELECT \
             {'nested': {'x': CAST(10 AS INTEGER)}}::STRUCT(nested STRUCT(x INTEGER)) AS data",
        )
        .await?;

    let v1 = DeployedSchema {
        model: "deep_model".to_string(),
        version: 1,
        deployed_at: chrono::Utc::now(),
        model_hash: "sha256:v1".to_string(),
        definition_sql: String::new(),
        columns: vec![DeployedColumn {
            name: "data".to_string(),
            data_type: "STRUCT(nested STRUCT(x INTEGER))".to_string(),
            nullable: true,
        }],
    };
    file_store.save_schema(&v1)?;

    // Infer v2: inner.x widened INTEGER→BIGINT, inner.y added
    let inferred = vec![DeployedColumn {
        name: "data".to_string(),
        data_type: "STRUCT(nested STRUCT(x BIGINT, y VARCHAR))".to_string(),
        nullable: true,
    }];

    let result = smelt_cli::migration::check_and_migrate(
        &backend,
        &file_store,
        "deep_model",
        "SELECT 1",
        "main",
        &inferred,
        false,
        false,
        false,
        &std::collections::HashMap::new(),
        &std::collections::HashMap::new(),
        None,
        &no_retry_policy(),
    )
    .await?;

    match &result {
        smelt_cli::migration::SchemaEvolutionResult::Migrated { statements, .. } => {
            assert!(!statements.is_empty(), "Should have migration statements");
        }
        other => panic!("Expected Migrated, got {:?}", other),
    }

    // Verify the nested type was widened
    let type_check = backend
        .execute_sql("SELECT typeof(data.nested.x) FROM main.deep_model LIMIT 1")
        .await?;
    let x_type = type_check[0]
        .column(0)
        .as_any()
        .downcast_ref::<arrow::array::StringArray>()
        .unwrap()
        .value(0);
    assert_eq!(x_type, "BIGINT", "data.nested.x should be BIGINT");

    // Verify the new field exists
    let y_check = backend
        .execute_sql("SELECT data.nested.y FROM main.deep_model LIMIT 1")
        .await;
    assert!(
        y_check.is_ok(),
        "data.nested.y should exist after migration"
    );

    Ok(())
}

/// 11c: Struct with array field widening
#[tokio::test]
async fn test_e2e_struct_with_array_field_widen() -> Result<()> {
    let (dir, backend) = setup_backend().await?;
    let file_store = FileStore::new(dir.path(), "dev", StateMode::Environments);

    backend
        .execute_sql(
            "CREATE TABLE main.struct_arr_model AS SELECT \
             {'items': [1, 2, 3]::INTEGER[]}::STRUCT(items INTEGER[]) AS data",
        )
        .await?;

    let v1 = DeployedSchema {
        model: "struct_arr_model".to_string(),
        version: 1,
        deployed_at: chrono::Utc::now(),
        model_hash: "sha256:v1".to_string(),
        definition_sql: String::new(),
        columns: vec![DeployedColumn {
            name: "data".to_string(),
            data_type: "STRUCT(items INTEGER[])".to_string(),
            nullable: true,
        }],
    };
    file_store.save_schema(&v1)?;

    // Infer v2: items widened INTEGER[] → BIGINT[]
    let inferred = vec![DeployedColumn {
        name: "data".to_string(),
        data_type: "STRUCT(items BIGINT[])".to_string(),
        nullable: true,
    }];

    let result = smelt_cli::migration::check_and_migrate(
        &backend,
        &file_store,
        "struct_arr_model",
        "SELECT 1",
        "main",
        &inferred,
        false,
        false,
        false,
        &std::collections::HashMap::new(),
        &std::collections::HashMap::new(),
        None,
        &no_retry_policy(),
    )
    .await?;

    match &result {
        smelt_cli::migration::SchemaEvolutionResult::Migrated { statements, .. } => {
            assert!(!statements.is_empty(), "Should have migration statements");
        }
        other => panic!("Expected Migrated, got {:?}", other),
    }

    // Verify the nested array type was widened
    let type_check = backend
        .execute_sql("SELECT typeof(data.items) FROM main.struct_arr_model LIMIT 1")
        .await?;
    let items_type = type_check[0]
        .column(0)
        .as_any()
        .downcast_ref::<arrow::array::StringArray>()
        .unwrap()
        .value(0);
    assert!(
        items_type.contains("BIGINT"),
        "data.items should be BIGINT[], got: {}",
        items_type
    );

    Ok(())
}

/// 11d: Map value struct field addition
#[tokio::test]
async fn test_e2e_map_value_struct_field_addition() -> Result<()> {
    let (dir, backend) = setup_backend().await?;
    let file_store = FileStore::new(dir.path(), "dev", StateMode::Environments);

    backend
        .execute_sql(
            "CREATE TABLE main.map_struct_model AS SELECT \
             MAP {'k1': {'a': 1}}::MAP(VARCHAR, STRUCT(a INTEGER)) AS lookup",
        )
        .await?;

    let v1 = DeployedSchema {
        model: "map_struct_model".to_string(),
        version: 1,
        deployed_at: chrono::Utc::now(),
        model_hash: "sha256:v1".to_string(),
        definition_sql: String::new(),
        columns: vec![DeployedColumn {
            name: "lookup".to_string(),
            data_type: "MAP(VARCHAR, STRUCT(a INTEGER))".to_string(),
            nullable: true,
        }],
    };
    file_store.save_schema(&v1)?;

    // Infer v2: struct inside map value gets new field
    let inferred = vec![DeployedColumn {
        name: "lookup".to_string(),
        data_type: "MAP(VARCHAR, STRUCT(a INTEGER, b TEXT))".to_string(),
        nullable: true,
    }];

    let result = smelt_cli::migration::check_and_migrate(
        &backend,
        &file_store,
        "map_struct_model",
        "SELECT 1",
        "main",
        &inferred,
        false,
        false,
        false,
        &std::collections::HashMap::new(),
        &std::collections::HashMap::new(),
        None,
        &no_retry_policy(),
    )
    .await?;

    match &result {
        smelt_cli::migration::SchemaEvolutionResult::Migrated { statements, .. } => {
            assert!(
                !statements.is_empty(),
                "Should have migration statements for map value struct field add"
            );
        }
        other => panic!("Expected Migrated, got {:?}", other),
    }

    // Verify the map value struct was updated
    let type_check = backend
        .execute_sql("SELECT typeof(lookup) FROM main.map_struct_model LIMIT 1")
        .await?;
    let lookup_type = type_check[0]
        .column(0)
        .as_any()
        .downcast_ref::<arrow::array::StringArray>()
        .unwrap()
        .value(0);
    assert!(
        lookup_type.contains("b"),
        "Map value struct should now have field b, got: {}",
        lookup_type
    );

    Ok(())
}

/// 11d: Map value widening (MAP(VARCHAR, INTEGER) → MAP(VARCHAR, BIGINT))
#[tokio::test]
async fn test_e2e_map_value_type_widening() -> Result<()> {
    let (dir, backend) = setup_backend().await?;
    let file_store = FileStore::new(dir.path(), "dev", StateMode::Environments);

    backend
        .execute_sql(
            "CREATE TABLE main.mapwiden_model AS SELECT \
             MAP {'a': 1, 'b': 2}::MAP(VARCHAR, INTEGER) AS lookup",
        )
        .await?;

    let v1 = DeployedSchema {
        model: "mapwiden_model".to_string(),
        version: 1,
        deployed_at: chrono::Utc::now(),
        model_hash: "sha256:v1".to_string(),
        definition_sql: String::new(),
        columns: vec![DeployedColumn {
            name: "lookup".to_string(),
            data_type: "MAP(VARCHAR, INTEGER)".to_string(),
            nullable: true,
        }],
    };
    file_store.save_schema(&v1)?;

    let inferred = vec![DeployedColumn {
        name: "lookup".to_string(),
        data_type: "MAP(VARCHAR, BIGINT)".to_string(),
        nullable: true,
    }];

    let result = smelt_cli::migration::check_and_migrate(
        &backend,
        &file_store,
        "mapwiden_model",
        "SELECT 1",
        "main",
        &inferred,
        false,
        false,
        false,
        &std::collections::HashMap::new(),
        &std::collections::HashMap::new(),
        None,
        &no_retry_policy(),
    )
    .await?;

    match &result {
        smelt_cli::migration::SchemaEvolutionResult::Migrated { statements, .. } => {
            assert!(
                !statements.is_empty(),
                "Should have ALTER statements for map value widening"
            );
        }
        other => panic!("Expected Migrated, got {:?}", other),
    }

    // Verify the value type was widened
    let type_check = backend
        .execute_sql("SELECT typeof(lookup) FROM main.mapwiden_model LIMIT 1")
        .await?;
    let type_str = type_check[0]
        .column(0)
        .as_any()
        .downcast_ref::<arrow::array::StringArray>()
        .unwrap()
        .value(0);
    assert!(
        type_str.contains("BIGINT"),
        "Map value should be BIGINT, got: {}",
        type_str
    );

    Ok(())
}
