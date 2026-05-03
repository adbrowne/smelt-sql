#![cfg(feature = "duckdb")]
//! Compile-and-execute integration test for the ecommerce example workspace.
//!
//! For each model: load workspace -> compile to DuckDB SQL -> create source
//! tables in DuckDB -> execute compiled SQL -> assert no errors.
//! This catches code-gen bugs that diagnostics alone miss.

use smelt_backend::Backend;
use smelt_backend_duckdb::DuckDbBackend;
use smelt_cli::{Config, LogicalGraph, ModelDiscovery, SourcesConfig, SqlCompiler};
use std::path::Path;
use tempfile::TempDir;

fn project_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples/ecommerce")
}

/// Create source tables and seed tables in DuckDB so compiled models can execute.
async fn seed_database(backend: &DuckDbBackend) -> anyhow::Result<()> {
    backend
        .execute_sql("CREATE SCHEMA IF NOT EXISTS raw")
        .await?;

    // raw.products
    backend
        .execute_sql(
            r#"
            CREATE TABLE raw.products AS
            SELECT * FROM (VALUES
                (1, 'Widget A', 'ELEC', 'premium', 9999, 4000, 500, false),
                (2, 'Widget B', 'CLTH', 'standard', 2999, 1200, 200, false),
                (3, 'E-Book C', 'BOOK', 'budget', 999, 100, 0, true)
            ) AS t(product_id, product_name, category_code, brand_tier, unit_price_cents, cost_cents, weight_grams, is_digital)
        "#,
        )
        .await?;

    // raw.orders
    backend
        .execute_sql(
            r#"
            CREATE TABLE raw.orders AS
            SELECT * FROM (VALUES
                (1, 100, '2024-01-15', 'completed', 'credit_card', 0),
                (2, 101, '2024-01-16', 'shipped', 'paypal', 10),
                (3, 100, '2024-01-17', 'cancelled', 'credit_card', 0)
            ) AS t(order_id, customer_id, order_date, status, payment_method, discount_pct)
        "#,
        )
        .await?;

    // raw.events
    backend
        .execute_sql(
            r#"
            CREATE TABLE raw.events AS
            SELECT * FROM (VALUES
                (1, 'v1', 'page_view', '2024-01-15 10:00:00'::TIMESTAMP, '/home'),
                (2, 'v1', 'page_view', '2024-01-15 10:05:00'::TIMESTAMP, '/products'),
                (3, 'v1', 'add_to_cart', '2024-01-15 10:06:00'::TIMESTAMP, '/products/1'),
                (4, 'v1', 'purchase', '2024-01-15 10:10:00'::TIMESTAMP, '/checkout'),
                (5, 'v2', 'page_view', '2024-01-15 14:00:00'::TIMESTAMP, '/home'),
                (6, 'v2', 'page_view', '2024-01-15 14:30:00'::TIMESTAMP, '/about')
            ) AS t(event_id, visitor_id, event_type, event_timestamp, page_url)
        "#,
        )
        .await?;

    // Seed tables (loaded as main.* tables)
    backend
        .execute_sql(
            r#"
            CREATE TABLE main.category_hierarchy AS
            SELECT * FROM (VALUES
                ('ELEC', 'Electronics', 'Technology'),
                ('CLTH', 'Clothing', 'Fashion'),
                ('HOME', 'Home & Kitchen', 'Home'),
                ('SPRT', 'Sports & Outdoors', 'Active'),
                ('BEAU', 'Beauty & Personal Care', 'Health')
            ) AS t(category_code, category_name, department)
        "#,
        )
        .await?;

    backend
        .execute_sql(
            r#"
            CREATE TABLE main.order_statuses AS
            SELECT * FROM (VALUES
                ('completed', 'Completed', true, true),
                ('shipped', 'Shipped', false, true),
                ('processing', 'Processing', false, true),
                ('cancelled', 'Cancelled', true, false),
                ('returned', 'Returned', true, false)
            ) AS t(status_code, status_label, is_terminal, is_successful)
        "#,
        )
        .await?;

    Ok(())
}

#[tokio::test]
async fn test_ecommerce_models_compile_and_execute() -> anyhow::Result<()> {
    let project_dir = project_dir();
    let config: Config =
        serde_yaml::from_str(&std::fs::read_to_string(project_dir.join("smelt.yml"))?)?;

    let discovery = ModelDiscovery::new(project_dir.clone(), config.paths.clone());
    let models = discovery.discover_models()?;
    let sources = SourcesConfig::load(&project_dir).ok();
    let seeds = smelt_core::discover_seed_infos(&project_dir, &config.paths);

    let default_target = config
        .targets
        .keys()
        .next()
        .map(|s| s.as_str())
        .unwrap_or("dev");

    // Build the logical graph and get execution order
    let graph = LogicalGraph::build(
        models.clone(),
        sources.as_ref(),
        &seeds,
        &config,
        default_target,
    )?;
    let exec_order = graph.execution_order()?;

    // Set up DuckDB
    let temp_dir = TempDir::new()?;
    let db_path = temp_dir.path().join("test.duckdb");
    let backend = DuckDbBackend::new(&db_path, "main").await?;

    seed_database(&backend).await?;

    // Compile and execute each model in dependency order
    let target = config.targets.get(default_target).unwrap();
    let compiler = SqlCompiler::new(config.clone(), target);

    let mut errors = Vec::new();
    for model_name in &exec_order {
        let model = match models.iter().find(|m| &m.name == model_name) {
            Some(m) => m,
            None => continue, // source node
        };

        match compiler.compile(model, "main") {
            Ok(compiled) => {
                // Execute as a CREATE TABLE for simplicity
                let create_sql = format!(
                    "CREATE OR REPLACE TABLE main.{} AS {}",
                    compiled.name, compiled.sql
                );
                if let Err(e) = backend.execute_sql(&create_sql).await {
                    errors.push(format!("{}: execution error: {}", model_name, e));
                }
            }
            Err(e) => {
                errors.push(format!("{}: compilation error: {}", model_name, e));
            }
        }
    }

    assert!(
        errors.is_empty(),
        "Ecommerce models failed compile-and-execute:\n  {}",
        errors.join("\n  ")
    );

    Ok(())
}
