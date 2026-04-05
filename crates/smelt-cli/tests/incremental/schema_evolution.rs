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
    let file_store = FileStore::new(dir.path());

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
    )
    .await?;

    match result {
        smelt_cli::migration::SchemaEvolutionResult::Migrated { statements } => {
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
    let file_store = FileStore::new(dir.path());

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
    )
    .await?;

    match result {
        smelt_cli::migration::SchemaEvolutionResult::Migrated { statements } => {
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
    let file_store = FileStore::new(dir.path());

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
    let file_store = FileStore::new(dir.path());

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
    let file_store = FileStore::new(dir.path());

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
    let file_store = FileStore::new(dir.path());

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
    )
    .await?;

    match result {
        smelt_cli::migration::SchemaEvolutionResult::Migrated { statements } => {
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
    let file_store = FileStore::new(dir.path());

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
    let file_store = FileStore::new(dir.path());

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
