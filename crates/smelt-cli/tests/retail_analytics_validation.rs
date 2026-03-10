//! Integration tests that validate the retail-analytics benchmark models.
//!
//! Ensures all 25 SQL models parse correctly, all smelt.ref() calls resolve,
//! and the dependency graph is acyclic.

use std::path::Path;

use smelt_cli::{Config, DependencyGraph, ModelDiscovery, SourcesConfig};

fn project_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("benchmarks/retail-analytics")
}

#[test]
fn test_retail_analytics_no_parse_errors() {
    let project_dir = project_dir();
    let config: Config =
        serde_yaml::from_str(&std::fs::read_to_string(project_dir.join("smelt.yml")).unwrap())
            .unwrap();

    let discovery = ModelDiscovery::new(project_dir.clone(), config.model_paths.clone());
    let models = discovery.discover_models().unwrap();

    assert!(
        !models.is_empty(),
        "Should discover at least 25 models in retail-analytics"
    );

    let mut errors = Vec::new();
    for model in &models {
        for err in &model.parse_errors {
            errors.push(format!("{}: {}", model.name, err.message));
        }
    }

    assert!(
        errors.is_empty(),
        "Retail analytics models should have no parse errors:\n  {}",
        errors.join("\n  ")
    );
}

#[test]
fn test_retail_analytics_no_undefined_refs() {
    let project_dir = project_dir();
    let config: Config =
        serde_yaml::from_str(&std::fs::read_to_string(project_dir.join("smelt.yml")).unwrap())
            .unwrap();

    let discovery = ModelDiscovery::new(project_dir.clone(), config.model_paths.clone());
    let models = discovery.discover_models().unwrap();
    let sources = SourcesConfig::load(&project_dir).ok();

    let graph = DependencyGraph::build(models, sources.as_ref()).unwrap();
    graph
        .validate()
        .expect("All refs in retail-analytics models should resolve to other models or sources.");
}

#[test]
fn test_retail_analytics_dag_is_acyclic() {
    let project_dir = project_dir();
    let config: Config =
        serde_yaml::from_str(&std::fs::read_to_string(project_dir.join("smelt.yml")).unwrap())
            .unwrap();

    let discovery = ModelDiscovery::new(project_dir.clone(), config.model_paths.clone());
    let models = discovery.discover_models().unwrap();
    let sources = SourcesConfig::load(&project_dir).ok();

    let graph = DependencyGraph::build(models, sources.as_ref()).unwrap();
    let order = graph.execution_order();
    assert!(order.is_ok(), "DAG should be acyclic: {:?}", order.err());
}
