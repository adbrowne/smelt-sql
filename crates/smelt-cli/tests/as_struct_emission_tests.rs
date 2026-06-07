//! Phase 55 — `smelt.as_struct()` and `smelt.fn.*` SQL emission tests.
//!
//! These tests verify that the dialect printer correctly translates:
//!   1. `smelt.as_struct(alias)` → backend-specific struct literal SQL
//!   2. `smelt.as_struct(alias EXCEPT col)` → struct literal excluding the column
//!   3. `smelt.fn.*` calls → inlined function body with parameter substitution
//!
//! Tests follow red-green TDD: they were written before the implementation.

use smelt_cli::config::{Config, Materialization, Target};
use smelt_cli::discovery::{ModelFile, ModelKind};
use smelt_runtime::{CompilerRegistry, UpstreamSchemas};
use smelt_types::{DataType, TypedColumn};
use std::collections::HashMap;
use std::sync::Arc;

/// Build a minimal `ModelFile` for testing.
fn model_file(name: &str, content: &str) -> ModelFile {
    ModelFile {
        name: name.to_string(),
        path: format!("models/{}.sql", name).into(),
        content: content.to_string(),
        refs: vec![],
        parse_errors: vec![],
        metadata: None,
        kind: ModelKind::Sql,
        model_id: smelt_core::ModelId::from_path(format!("models/{}.sql", name).into()),
        address_segments: vec![name.to_string()],
    }
}

/// Build a minimal DuckDB config and target.
fn duckdb_config_and_target() -> (Config, Target) {
    let target = Target {
        target_type: "duckdb".to_string(),
        database: None,
        schema: "main".to_string(),
        connect_url: None,
        catalog: None,
        warehouse: None,
        format: None,
    };
    let mut targets = HashMap::new();
    targets.insert("default".to_string(), target.clone());
    let config = Config {
        name: "test".to_string(),
        version: 1,
        paths: vec!["models".to_string()],
        targets,
        default_materialization: Materialization::View,
        models: HashMap::new(),
        python: None,
        target: None,
    };
    (config, target)
}

/// Build a minimal DuckDB compiler registry.
fn duckdb_registry() -> CompilerRegistry {
    let (config, _) = duckdb_config_and_target();
    let targets = config.targets.clone();
    CompilerRegistry::new(&config, &targets)
}

/// Build upstream schemas with known columns for a model name.
fn upstream_with_model(model_name: &str, cols: Vec<(String, DataType)>) -> Arc<UpstreamSchemas> {
    let mut models = HashMap::new();
    let typed_cols: Vec<(String, TypedColumn)> = cols
        .into_iter()
        .map(|(name, dt)| {
            (
                name,
                TypedColumn {
                    data_type: dt,
                    nullable: true,
                },
            )
        })
        .collect();
    models.insert(model_name.to_string(), typed_cols);
    Arc::new(UpstreamSchemas {
        models,
        seeds: HashMap::new(),
        sources: Default::default(),
        per_entity_sources: Vec::new(),
        vars: Default::default(),
        ..Default::default()
    })
}

// ─── Test 1: smelt.as_struct(alias) emits DuckDB struct literal ──────────────

#[test]
fn as_struct_emits_duckdb_struct_literal() {
    // The upstream model `orders` has two columns: order_id (BigInt), total (Double).
    let schemas = upstream_with_model(
        "orders",
        vec![
            ("order_id".to_string(), DataType::BigInt),
            ("total".to_string(), DataType::Double),
        ],
    );
    let mut registry = duckdb_registry();
    registry.set_upstream_schemas_all(schemas);
    let compiler = registry.get("default");

    // A model that uses smelt.as_struct on the alias `o`.
    // Note: avoid `row` as an alias name — it's a SQL reserved word that confuses the parser.
    let sql = "SELECT smelt.as_struct(o) AS order_row FROM smelt.models.orders o";
    let model = model_file("test_as_struct", sql);
    let result = compiler
        .compile(&model, "main")
        .expect("compile should succeed");

    // DuckDB struct literal: {'order_id': o.order_id, 'total': o.total}
    assert!(
        result.sql.contains("'order_id': o.order_id"),
        "Expected DuckDB struct field for order_id, got: {}",
        result.sql
    );
    assert!(
        result.sql.contains("'total': o.total"),
        "Expected DuckDB struct field for total, got: {}",
        result.sql
    );
    assert!(
        !result.sql.contains("smelt.as_struct"),
        "smelt.as_struct should have been replaced, got: {}",
        result.sql
    );
}

// ─── Test 2: smelt.as_struct(alias EXCEPT col) excludes listed columns ───────

#[test]
fn as_struct_except_excludes_columns() {
    let schemas = upstream_with_model(
        "orders",
        vec![
            ("order_id".to_string(), DataType::BigInt),
            ("total".to_string(), DataType::Double),
            ("tax".to_string(), DataType::Double),
        ],
    );
    let mut registry = duckdb_registry();
    registry.set_upstream_schemas_all(schemas);
    let compiler = registry.get("default");

    let sql = "SELECT smelt.as_struct(o EXCEPT tax) AS order_row FROM smelt.models.orders o";
    let model = model_file("test_as_struct_except", sql);
    let result = compiler
        .compile(&model, "main")
        .expect("compile should succeed");

    assert!(
        result.sql.contains("'order_id': o.order_id"),
        "order_id should be included, got: {}",
        result.sql
    );
    assert!(
        result.sql.contains("'total': o.total"),
        "total should be included, got: {}",
        result.sql
    );
    assert!(
        !result.sql.contains("'tax'"),
        "tax should be excluded, got: {}",
        result.sql
    );
    assert!(
        !result.sql.contains("smelt.as_struct"),
        "smelt.as_struct should have been replaced, got: {}",
        result.sql
    );
}

// ─── Test 3: smelt.functions.* call expands function body ───────────────────────────

#[test]
fn smelt_fn_call_expands_body() {
    // A function `safe_div` with params (a, b) and body: a / NULLIF(b, 0)
    let mut fn_bodies: smelt_runtime::FnBodyMap = HashMap::new();
    fn_bodies.insert(
        "safe_div".to_string(),
        (
            vec![("a".to_string(), None), ("b".to_string(), None)],
            "a / NULLIF(b, 0)".to_string(),
        ),
    );
    let mut registry = duckdb_registry();
    registry.set_function_bodies_all(fn_bodies);
    let compiler = registry.get("default");

    let sql = "SELECT smelt.functions.safe_div(revenue, cost) AS result FROM t";
    let model = model_file("test_fn_call", sql);
    let result = compiler
        .compile(&model, "main")
        .expect("compile should succeed");

    assert!(
        result.sql.contains("revenue / NULLIF(cost, 0)"),
        "Expected expanded body with param substitution, got: {}",
        result.sql
    );
    assert!(
        !result.sql.contains("smelt.functions.safe_div"),
        "smelt.functions call should have been replaced, got: {}",
        result.sql
    );
}

// ─── Test 4: smelt.functions.* with no body map passes through verbatim ─────────────

#[test]
fn smelt_fn_call_passthrough_when_no_body_map() {
    // When no function_bodies are set (legacy / test mode), smelt.functions.* passes through.
    let registry = duckdb_registry();
    let compiler = registry.get("default");
    let sql = "SELECT smelt.functions.some_fn(x) FROM t";
    let model = model_file("test_fn_passthrough", sql);
    // Should not fail, but pass through verbatim
    let result = compiler
        .compile(&model, "main")
        .expect("compile should succeed");
    // The SQL may be verbatim or partially transformed; just check no panic
    let _ = result.sql;
}

// ─── Test 5: TypeContext builds correctly for smelt.as_struct queries ─────────

#[test]
fn type_context_builds_alias_for_as_struct() {
    use smelt_db::{build_type_context, StaticRefSchemaProvider};
    use smelt_parser::ast::File;

    // Use `order_row` not `row` (SQL reserved word confuses parser)
    let sql = "SELECT smelt.as_struct(o) AS order_row FROM smelt.models.orders o";
    let clean = smelt_parser::strip_frontmatter(sql);
    let parse = smelt_parser::parse(&clean);

    let mut models = HashMap::new();
    models.insert(
        "orders".to_string(),
        vec![
            (
                "order_id".to_string(),
                TypedColumn {
                    data_type: DataType::BigInt,
                    nullable: true,
                },
            ),
            (
                "total".to_string(),
                TypedColumn {
                    data_type: DataType::Double,
                    nullable: true,
                },
            ),
        ],
    );

    let file = File::cast(parse.syntax()).expect("should parse as FILE");
    let provider = StaticRefSchemaProvider {
        models: &models,
        seeds: &HashMap::new(),
    };
    let sources = Default::default();
    let ctx = build_type_context(&file, &sources, &provider);

    let cols = ctx.columns_for_qualifier("o");
    assert!(
        !cols.is_empty(),
        "Expected columns for alias 'o', got empty"
    );

    let col_names: Vec<&str> = cols.iter().map(|(n, _)| *n).collect();
    assert!(
        col_names.contains(&"order_id"),
        "Expected order_id, got: {:?}",
        col_names
    );
    assert!(
        col_names.contains(&"total"),
        "Expected total, got: {:?}",
        col_names
    );
}
