#![cfg(feature = "duckdb")]
//! Integration tests for multi-engine (cross-backend) ref resolution.
//!
//! These tests verify that:
//! 1. `find_cross_backend_edges()` correctly detects cross-engine dependencies
//! 2. The SqlCompiler emits `read_parquet()` for cross-engine refs
//! 3. DuckDB can execute queries that read Parquet via cross-engine ref resolution

use smelt_backend::Backend;
use smelt_backend_duckdb::DuckDbBackend;
use smelt_cli::{
    compiler::SqlCompiler,
    config::{Config, Materialization, ModelConfig, Target},
    discovery::{ModelFile, ModelKind, RefInfo},
    LogicalGraph, ModelDiscovery,
};
use std::collections::HashMap;
use tempfile::TempDir;

// ─── Helper Functions ──────────────────────────────────────────────

fn make_duckdb_target(schema: &str) -> Target {
    Target {
        target_type: "duckdb".to_string(),
        database: Some("test.duckdb".to_string()),
        schema: schema.to_string(),
        connect_url: None,
        catalog: None,
        warehouse: None,
        format: None,
    }
}

fn make_spark_target(schema: &str, warehouse: &str) -> Target {
    Target {
        target_type: "spark".to_string(),
        database: None,
        schema: schema.to_string(),
        connect_url: None,
        catalog: None,
        warehouse: Some(warehouse.to_string()),
        format: None,
    }
}

fn make_config_with_targets(targets: HashMap<String, Target>) -> Config {
    Config {
        name: "test".to_string(),
        version: 1,
        paths: vec!["models".to_string()],
        targets,
        default_materialization: Materialization::View,
        models: HashMap::new(),
        python: None,
    }
}

fn extract_refs_from_sql(sql: &str) -> Vec<RefInfo> {
    let parse = smelt_parser::parse(sql);
    if let Some(file) = smelt_parser::File::cast(parse.syntax()) {
        smelt_core::extract_refs(&file)
    } else {
        Vec::new()
    }
}

fn make_model_file(name: &str, sql: &str) -> ModelFile {
    ModelFile {
        name: name.to_string(),
        path: format!("models/{}.sql", name).into(),
        content: sql.to_string(),
        refs: extract_refs_from_sql(sql),
        parse_errors: Vec::new(),
        metadata: None,
        kind: ModelKind::Sql,
        model_id: smelt_core::ModelId::from_path(format!("models/{}.sql", name).into()),
        address_segments: vec![name.to_string()],
    }
}

// ─── LogicalGraph Cross-Engine Edge Detection ──────────────────────

#[test]
fn test_logical_graph_cross_engine_edges() {
    let upstream_sql = "SELECT 1 as id, 'hello' as name";
    let downstream_sql = "SELECT * FROM smelt.upstream_model";

    let models = vec![
        make_model_file("upstream_model", upstream_sql),
        make_model_file("downstream_model", downstream_sql),
    ];

    let mut targets = HashMap::new();
    targets.insert("duckdb_local".to_string(), make_duckdb_target("main"));
    targets.insert(
        "spark_local".to_string(),
        make_spark_target("default", "/tmp/warehouse"),
    );

    let mut config = make_config_with_targets(targets);
    config.models.insert(
        "upstream_model".to_string(),
        ModelConfig {
            materialization: None,
            incremental: None,
            tags: vec![],
            target: Some("spark_local".to_string()),
        },
    );

    let graph = LogicalGraph::build(models, None, &[], &config, "duckdb_local")
        .expect("Graph build should succeed");

    let edges = graph.find_cross_backend_edges();
    assert_eq!(edges.len(), 1, "Should detect one cross-engine edge");
    assert_eq!(edges[0].upstream, "upstream_model");
    assert_eq!(edges[0].downstream, "downstream_model");
    assert_eq!(edges[0].upstream_target, "spark_local");
    assert_eq!(edges[0].downstream_target, "duckdb_local");
}

#[test]
fn test_logical_graph_no_cross_engine_edges_same_target() {
    let models = vec![
        make_model_file("model_a", "SELECT 1 as id"),
        make_model_file("model_b", "SELECT * FROM smelt.model_a"),
    ];

    let mut targets = HashMap::new();
    targets.insert("duckdb_local".to_string(), make_duckdb_target("main"));

    let config = make_config_with_targets(targets);

    let graph = LogicalGraph::build(models, None, &[], &config, "duckdb_local")
        .expect("Graph build should succeed");

    let edges = graph.find_cross_backend_edges();
    assert!(
        edges.is_empty(),
        "Should detect no cross-engine edges when all models on same target"
    );
}

// ─── Compiler Cross-Engine Ref Resolution ──────────────────────────

#[test]
fn test_compiler_cross_engine_ref_emits_read_parquet() {
    let sql = "SELECT * FROM smelt.spark_model";
    let model = make_model_file("duckdb_consumer", sql);

    let mut targets = HashMap::new();
    targets.insert("duckdb_local".to_string(), make_duckdb_target("main"));
    let config = make_config_with_targets(targets);

    let target = make_duckdb_target("main");
    let mut compiler = SqlCompiler::new(config, &target);

    let mut cross_refs = HashMap::new();
    cross_refs.insert(
        "spark_model".to_string(),
        "read_parquet('/warehouse/default/spark_model/**/*.parquet', hive_partitioning=true)"
            .to_string(),
    );
    compiler.set_cross_engine_refs(cross_refs);

    let compiled = compiler
        .compile(&model, "main")
        .expect("Compile should succeed");

    assert!(
        compiled.sql.contains("read_parquet("),
        "Should contain read_parquet, got: {}",
        compiled.sql
    );
    assert!(
        compiled.sql.contains("spark_model/**/*.parquet"),
        "Should contain parquet glob path, got: {}",
        compiled.sql
    );
    assert!(
        !compiled.sql.contains("main.spark_model"),
        "Should not contain schema-qualified ref for cross-engine model, got: {}",
        compiled.sql
    );
}

#[test]
fn test_compiler_cross_engine_mixed_refs() {
    let sql = r#"
SELECT a.id, b.name
FROM smelt.local_model a
JOIN smelt.spark_model b ON a.id = b.id
"#;
    let model = make_model_file("mixed_consumer", sql);

    let mut targets = HashMap::new();
    targets.insert("duckdb_local".to_string(), make_duckdb_target("main"));
    let config = make_config_with_targets(targets);

    let target = make_duckdb_target("main");
    let mut compiler = SqlCompiler::new(config, &target);

    let mut cross_refs = HashMap::new();
    cross_refs.insert(
        "spark_model".to_string(),
        "read_parquet('/warehouse/default/spark_model/**/*.parquet', hive_partitioning=true)"
            .to_string(),
    );
    compiler.set_cross_engine_refs(cross_refs);

    let compiled = compiler
        .compile(&model, "main")
        .expect("Compile should succeed");

    // Local ref should resolve normally
    assert!(
        compiled.sql.contains("main.local_model"),
        "Local ref should resolve to schema.model, got: {}",
        compiled.sql
    );

    // Cross-engine ref should resolve to read_parquet
    assert!(
        compiled.sql.contains("read_parquet("),
        "Cross-engine ref should resolve to read_parquet, got: {}",
        compiled.sql
    );
}

// ─── DuckDB Integration: Parquet Read via Cross-Engine Ref ─────────

#[tokio::test]
async fn test_duckdb_reads_parquet_via_cross_engine_ref() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let db_path = temp_dir.path().join("test.duckdb");
    let warehouse_path = temp_dir.path().join("warehouse");

    // Create warehouse directory structure: warehouse/default/spark_model/
    let parquet_dir = warehouse_path.join("default").join("spark_model");
    std::fs::create_dir_all(&parquet_dir).expect("Failed to create warehouse dirs");

    // Use DuckDB to write Parquet files simulating Spark output
    let writer_backend = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("Failed to create DuckDB backend");

    // Write a parquet file with test data
    let parquet_path = parquet_dir.join("part-00000.parquet");
    writer_backend
        .execute_sql(&format!(
            "COPY (
                SELECT * FROM (VALUES
                    (1, 'Alice', 100),
                    (2, 'Bob', 200),
                    (3, 'Charlie', 300)
                ) AS t(id, name, amount)
            ) TO '{}' (FORMAT PARQUET)",
            parquet_path.display()
        ))
        .await
        .expect("Failed to write parquet");

    // Now query using read_parquet (simulating cross-engine ref resolution)
    let parquet_expr = format!(
        "read_parquet('{}/**/*.parquet', hive_partitioning=true)",
        parquet_dir.display()
    );

    let result = writer_backend
        .execute_sql(&format!("SELECT COUNT(*) as cnt FROM {}", parquet_expr))
        .await
        .expect("Failed to query parquet");

    let count = result[0]
        .column(0)
        .as_any()
        .downcast_ref::<arrow::array::Int64Array>()
        .expect("Expected Int64Array")
        .value(0);
    assert_eq!(count, 3, "Expected 3 rows from parquet read");

    // Verify actual data content
    let result = writer_backend
        .execute_sql(&format!(
            "SELECT SUM(amount) as total FROM {}",
            parquet_expr
        ))
        .await
        .expect("Failed to query parquet sum");

    let total: i128 = result[0]
        .column(0)
        .as_any()
        .downcast_ref::<arrow::array::Decimal128Array>()
        .expect("Expected Decimal128Array for SUM")
        .value(0);
    assert_eq!(total, 600, "Expected total amount of 600");
}

#[tokio::test]
async fn test_cross_engine_end_to_end_compile_and_execute() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let db_path = temp_dir.path().join("test.duckdb");
    let warehouse_path = temp_dir.path().join("warehouse");

    // Create warehouse directory with simulated Spark output
    let parquet_dir = warehouse_path.join("default").join("visitor_daily");
    std::fs::create_dir_all(&parquet_dir).expect("Failed to create warehouse dirs");

    let backend = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("Failed to create backend");

    // Write Parquet simulating Spark incremental output (batch 1: Jan 1-2)
    let batch1_path = parquet_dir.join("batch1.parquet");
    backend
        .execute_sql(&format!(
            "COPY (
                SELECT * FROM (VALUES
                    ('2024-01-01'::DATE, 'US', 100, 50),
                    ('2024-01-01'::DATE, 'UK', 80, 30),
                    ('2024-01-02'::DATE, 'US', 120, 60),
                    ('2024-01-02'::DATE, 'UK', 90, 40)
                ) AS t(visit_date, country, visitors, sessions)
            ) TO '{}' (FORMAT PARQUET)",
            batch1_path.display()
        ))
        .await
        .expect("Failed to write batch1 parquet");

    // Compile a model that references the "spark" model
    let model_sql = r#"SELECT
    visit_date,
    country,
    visitors,
    sessions,
    ROUND(CAST(sessions AS DOUBLE) / CAST(visitors AS DOUBLE), 2) as session_rate
FROM smelt.visitor_daily"#;

    let model = make_model_file("daily_metrics", model_sql);

    let mut targets = HashMap::new();
    targets.insert("duckdb_local".to_string(), make_duckdb_target("main"));
    let config = make_config_with_targets(targets);

    let target = make_duckdb_target("main");
    let mut compiler = SqlCompiler::new(config, &target);

    let mut cross_refs = HashMap::new();
    cross_refs.insert(
        "visitor_daily".to_string(),
        format!(
            "read_parquet('{}/**/*.parquet', hive_partitioning=true)",
            parquet_dir.display()
        ),
    );
    compiler.set_cross_engine_refs(cross_refs);

    let compiled = compiler
        .compile(&model, "main")
        .expect("Compile should succeed");

    // Execute the compiled SQL
    backend
        .execute_sql(&format!(
            "CREATE TABLE main.daily_metrics AS {}",
            compiled.sql
        ))
        .await
        .expect("Failed to create table from compiled SQL");

    let count = backend
        .get_row_count("main", "daily_metrics")
        .await
        .expect("Failed to get row count");
    assert_eq!(count, 4, "Expected 4 rows from batch 1");

    // Simulate incremental: add batch 2 (Jan 3)
    let batch2_path = parquet_dir.join("batch2.parquet");
    backend
        .execute_sql(&format!(
            "COPY (
                SELECT * FROM (VALUES
                    ('2024-01-03'::DATE, 'US', 150, 70),
                    ('2024-01-03'::DATE, 'UK', 100, 50)
                ) AS t(visit_date, country, visitors, sessions)
            ) TO '{}' (FORMAT PARQUET)",
            batch2_path.display()
        ))
        .await
        .expect("Failed to write batch2 parquet");

    // Re-run: drop and recreate (simulating full refresh on downstream)
    backend
        .execute_sql("DROP TABLE IF EXISTS main.daily_metrics")
        .await
        .expect("Failed to drop table");
    backend
        .execute_sql(&format!(
            "CREATE TABLE main.daily_metrics AS {}",
            compiled.sql
        ))
        .await
        .expect("Failed to recreate table");

    let count = backend
        .get_row_count("main", "daily_metrics")
        .await
        .expect("Failed to get row count");
    assert_eq!(
        count, 6,
        "Expected 6 rows after batch 2 (both parquet files read)"
    );

    // Verify data correctness
    let result = backend
        .execute_sql("SELECT COUNT(*) FROM main.daily_metrics WHERE visit_date = '2024-01-03'")
        .await
        .expect("Failed to query batch 2 data");

    let batch2_count = result[0]
        .column(0)
        .as_any()
        .downcast_ref::<arrow::array::Int64Array>()
        .expect("Expected Int64Array")
        .value(0);
    assert_eq!(batch2_count, 2, "Expected 2 rows for Jan 3 from batch 2");
}

// ─── Multi-Engine Example Project Validation ───────────────────────

#[test]
fn test_multi_engine_example_parses() {
    let project_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples/multi_engine");

    if !project_dir.exists() {
        // Skip if example not present (e.g., in CI without full checkout)
        return;
    }

    let config_path = project_dir.join("smelt.yml");
    if !config_path.exists() {
        return;
    }

    let config: Config =
        serde_yaml::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();

    let discovery = ModelDiscovery::new(project_dir.clone(), config.paths.clone());
    let models = discovery.discover_models().unwrap();

    assert!(
        !models.is_empty(),
        "Should discover models in multi_engine example"
    );

    // Verify no parse errors
    let mut errors = Vec::new();
    for model in &models {
        for err in &model.parse_errors {
            errors.push(format!("{}: {}", model.name, err.message));
        }
    }

    assert!(
        errors.is_empty(),
        "Multi-engine models should have no parse errors:\n  {}",
        errors.join("\n  ")
    );

    // Build graph and verify cross-engine edges exist
    let graph = LogicalGraph::build(models, None, &[], &config, "duckdb_local")
        .expect("Graph build should succeed for multi-engine example");

    let edges = graph.find_cross_backend_edges();
    assert!(
        !edges.is_empty(),
        "Multi-engine example should have cross-engine edges"
    );
}
