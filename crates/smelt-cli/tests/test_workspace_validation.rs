//! Integration test that validates the test-workspace has no unexpected errors.
//!
//! This ensures that changes to the parser, discovery, or frontmatter handling
//! don't silently introduce errors in our example models.

use std::path::Path;

use smelt_cli::{Config, LogicalGraph, ModelDiscovery, SourcesConfig};

fn project_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples/test_workspace")
}

#[test]
fn test_workspace_no_parse_errors() {
    let project_dir = project_dir();
    let config: Config =
        serde_yaml::from_str(&std::fs::read_to_string(project_dir.join("smelt.yml")).unwrap())
            .unwrap();

    let discovery = ModelDiscovery::new(project_dir.clone(), config.paths.clone());
    let models = discovery.discover_models().unwrap();

    let mut errors = Vec::new();
    for model in &models {
        for err in &model.parse_errors {
            errors.push(format!("{}: {}", model.name, err.message));
        }
    }

    assert!(
        errors.is_empty(),
        "Test workspace models should have no parse errors:\n  {}",
        errors.join("\n  ")
    );
}

#[test]
fn test_workspace_no_undefined_refs() {
    let project_dir = project_dir();
    let config: Config =
        serde_yaml::from_str(&std::fs::read_to_string(project_dir.join("smelt.yml")).unwrap())
            .unwrap();

    let discovery = ModelDiscovery::new(project_dir.clone(), config.paths.clone());
    let mut models = discovery.discover_models().unwrap();

    // Discover Python models
    let python_files = discovery.discover_python_files().unwrap();
    if !python_files.is_empty() {
        let python_models = smelt_cli::discover_python_models(
            &python_files,
            &models,
            &config,
            &project_dir,
            config.python.as_deref(),
        )
        .unwrap();
        models.extend(python_models);
    }

    let sources = SourcesConfig::load(&project_dir).ok();
    let seeds = smelt_core::discover_seed_infos(&project_dir, &config.paths);

    let default_target = config
        .targets
        .keys()
        .next()
        .map(|s| s.as_str())
        .unwrap_or("dev");
    let graph =
        LogicalGraph::build(models, sources.as_ref(), &seeds, &config, default_target).unwrap();
    graph.validate().expect(
        "All refs in test_workspace models should resolve.\n\
         If this fails, a model references an undefined model or source.",
    );
}
