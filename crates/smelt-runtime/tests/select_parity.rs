//! Selection-parity tests. Exercise `smelt_runtime::select_executable_models`
//! against small in-memory dependency graphs, anchoring Phase 3 invariants:
//!
//! - `materialization: test` models are filtered out (today's UI test-model
//!   panic regression test).
//! - Generator files (`.gen.sql` / `name.gen`) are filtered out.
//! - Selectors compose with excludes consistently.
//! - Per-model `target:` metadata overrides win over the request default.
//! - Cross-engine edges surface when models on different backends reference
//!   each other.

use smelt_core::config::{Config, Materialization, Target};
use smelt_core::graph::DependencyGraph;
use smelt_core::metadata::ModelMetadata;
use smelt_runtime::{select_executable_models, SelectionRequest};
use std::collections::HashMap;

fn make_target(name: &str) -> Target {
    Target {
        target_type: name.to_string(),
        database: Some(format!("{name}.duckdb")),
        schema: "main".to_string(),
        connect_url: None,
        catalog: None,
        warehouse: None,
        format: None,
    }
}

fn config_with_targets(targets: &[&str]) -> Config {
    let mut t = HashMap::new();
    for name in targets {
        t.insert((*name).to_string(), make_target(name));
    }
    Config {
        name: "select_parity".to_string(),
        version: 1,
        paths: vec!["models".to_string()],
        targets: t,
        default_materialization: Materialization::View,
        models: HashMap::new(),
        python: None,
        target: None,
    }
}

fn make_model(name: &str, sql: &str, metadata: Option<ModelMetadata>) -> smelt_core::ModelFile {
    let parse = smelt_parser::parse(sql);
    let refs = smelt_parser::ast::File::cast(parse.syntax())
        .map(|f| smelt_core::extract_refs(&f))
        .unwrap_or_default();
    let path = std::path::PathBuf::from(format!("models/{name}.sql"));
    smelt_core::ModelFile {
        name: name.to_string(),
        path: path.clone(),
        content: sql.to_string(),
        refs,
        parse_errors: Vec::new(),
        metadata: metadata.map(Box::new),
        kind: smelt_core::ModelKind::Sql,
        model_id: smelt_core::ModelId::from_path(path),
        address_segments: vec![name.to_string()],
    }
}

fn make_test_model(name: &str, target_model: &str) -> smelt_core::ModelFile {
    let metadata = ModelMetadata {
        materialization: Some(Materialization::Test),
        test: Some(smelt_core::metadata::TestConfig {
            model: target_model.to_string(),
            ..Default::default()
        }),
        ..Default::default()
    };
    make_model(
        name,
        &format!("SELECT * FROM smelt.{target_model}"),
        Some(metadata),
    )
}

#[test]
fn test_test_models_excluded() {
    // Regular model + a test that targets it.
    let models = vec![
        make_model("orders", "SELECT 1", None),
        make_test_model("orders_no_nulls", "orders"),
    ];
    let graph = DependencyGraph::build(models, None).unwrap();
    let config = config_with_targets(&["default"]);

    let request = SelectionRequest {
        select: vec![],
        exclude: vec![],
        target: "default".to_string(),
    };
    let plan = select_executable_models(&graph, &config, &request).unwrap();

    assert!(
        plan.ordered_models.contains(&"orders".to_string()),
        "expected regular model in selection, got: {:?}",
        plan.ordered_models
    );
    assert!(
        !plan.ordered_models.contains(&"orders_no_nulls".to_string()),
        "test model should be filtered out, got: {:?}",
        plan.ordered_models
    );
}

#[test]
fn test_gen_files_excluded() {
    // A "regular" model and a generator file (`.gen` name suffix). The
    // generator file's body is meta-language, not SQL — it must never reach
    // the execute path.
    let mut gen = make_model("cohorts.gen", "SELECT 1", None);
    gen.path = std::path::PathBuf::from("models/cohorts.gen.sql");

    let models = vec![make_model("orders", "SELECT 1", None), gen];
    let graph = DependencyGraph::build(models, None).unwrap();
    let config = config_with_targets(&["default"]);

    let request = SelectionRequest {
        select: vec![],
        exclude: vec![],
        target: "default".to_string(),
    };
    let plan = select_executable_models(&graph, &config, &request).unwrap();

    assert!(
        plan.ordered_models.contains(&"orders".to_string()),
        "regular model should be present, got: {:?}",
        plan.ordered_models
    );
    assert!(
        !plan.ordered_models.contains(&"cohorts.gen".to_string()),
        "generator file should be filtered, got: {:?}",
        plan.ordered_models
    );
}

#[test]
fn test_selector_parsing_consistent() {
    // Selectors should resolve to the same model set regardless of consumer.
    // Using a plain model-name selector here; the broader selector grammar
    // is tested in `smelt-core`.
    let models = vec![
        make_model("a", "SELECT 1", None),
        make_model("b", "SELECT 1", None),
    ];
    let graph = DependencyGraph::build(models, None).unwrap();
    let config = config_with_targets(&["default"]);

    let request = SelectionRequest {
        select: vec!["a".to_string()],
        exclude: vec![],
        target: "default".to_string(),
    };
    let plan = select_executable_models(&graph, &config, &request).unwrap();
    assert_eq!(plan.ordered_models, vec!["a".to_string()]);
}

#[test]
fn test_excludes_combined_with_selects() {
    let models = vec![
        make_model("a", "SELECT 1", None),
        make_model("b", "SELECT 1", None),
        make_model("c", "SELECT 1", None),
    ];
    let graph = DependencyGraph::build(models, None).unwrap();
    let config = config_with_targets(&["default"]);

    // Select all, exclude b.
    let request = SelectionRequest {
        select: vec![],
        exclude: vec!["b".to_string()],
        target: "default".to_string(),
    };
    let plan = select_executable_models(&graph, &config, &request).unwrap();
    let names: std::collections::HashSet<_> = plan.ordered_models.into_iter().collect();
    assert!(names.contains("a"));
    assert!(names.contains("c"));
    assert!(!names.contains("b"));
}

#[test]
fn test_per_model_target_assignment() {
    // Model with no metadata → default target. Model with `target:` metadata
    // override → override target.
    let plain = make_model("plain", "SELECT 1", None);
    let overridden = make_model(
        "overridden",
        "SELECT 1",
        Some(ModelMetadata {
            target: Some("warehouse".to_string()),
            ..Default::default()
        }),
    );

    let models = vec![plain, overridden];
    let graph = DependencyGraph::build(models, None).unwrap();
    let config = config_with_targets(&["default", "warehouse"]);

    let request = SelectionRequest {
        select: vec![],
        exclude: vec![],
        target: "default".to_string(),
    };
    let plan = select_executable_models(&graph, &config, &request).unwrap();

    assert_eq!(
        plan.target_assignments.get("plain"),
        Some(&"default".to_string())
    );
    assert_eq!(
        plan.target_assignments.get("overridden"),
        Some(&"warehouse".to_string())
    );
}
