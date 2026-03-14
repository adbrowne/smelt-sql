pub mod config_gen;
pub mod graph_builder;
pub mod python_templates;
pub mod sql_templates;

use anyhow::Result;
use rand::prelude::*;
use rand_chacha::ChaChaRng;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

pub use graph_builder::{GraphSpec, ModelSpec, ModelType};

/// Generated workspace containing models and config files.
pub struct GeneratedWorkspace {
    pub dir: TempDir,
    pub model_specs: Vec<ModelSpec>,
    pub sql_contents: Vec<(String, String)>, // (name, content)
}

impl GeneratedWorkspace {
    /// Get the root path of the workspace.
    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    /// Get the models directory path.
    pub fn models_path(&self) -> PathBuf {
        self.dir.path().join("models")
    }
}

/// Generate a complete workspace with models and configuration.
///
/// Creates a temporary directory with:
/// - `models/` containing SQL and Python model files
/// - `smelt.yml` project configuration
/// - `sources.yml` source definitions
pub fn generate_workspace(spec: &GraphSpec) -> Result<GeneratedWorkspace> {
    let dir = TempDir::new()?;
    let models_dir = dir.path().join("models");
    std::fs::create_dir_all(&models_dir)?;

    // Build the dependency graph
    let model_specs = graph_builder::build_graph(spec);

    // Create seeded RNG for template selection
    let mut rng = ChaChaRng::seed_from_u64(spec.seed);

    let mut sql_contents = Vec::new();

    // Generate model files
    for model_spec in &model_specs {
        match model_spec.model_type {
            ModelType::Sql => {
                let content = sql_templates::generate_sql(&mut rng, model_spec);
                let file_path = models_dir.join(format!("{}.sql", model_spec.name));
                std::fs::write(&file_path, &content)?;
                sql_contents.push((model_spec.name.clone(), content));
            }
            ModelType::Python => {
                let content = python_templates::generate_python(&mut rng, model_spec);
                let file_path = models_dir.join(format!("{}.py", model_spec.name));
                std::fs::write(&file_path, &content)?;
            }
        }
    }

    // Generate configuration files
    let smelt_yml = config_gen::generate_smelt_yml(spec, &model_specs);
    std::fs::write(dir.path().join("smelt.yml"), smelt_yml)?;

    let sources_yml = config_gen::generate_sources_yml(spec);
    std::fs::write(dir.path().join("sources.yml"), sources_yml)?;

    Ok(GeneratedWorkspace {
        dir,
        model_specs,
        sql_contents,
    })
}
