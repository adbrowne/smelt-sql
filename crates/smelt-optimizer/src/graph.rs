use std::collections::HashMap;

use crate::types::IncrementalConfig;

/// Information about a single model for the optimizer.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ModelInfo {
    pub name: String,
    /// Raw SQL content (including any frontmatter).
    pub sql: String,
    /// Model names this model references via smelt.ref().
    pub refs: Vec<String>,
    /// Incremental configuration parsed from frontmatter, if any.
    pub incremental_config: Option<IncrementalConfig>,
}

/// A collection of models the optimizer can analyze.
#[derive(Debug, Clone)]
pub struct ModelGraph {
    models: HashMap<String, ModelInfo>,
}

impl ModelGraph {
    pub fn new() -> Self {
        Self {
            models: HashMap::new(),
        }
    }

    pub fn add_model(&mut self, model: ModelInfo) {
        self.models.insert(model.name.clone(), model);
    }

    pub fn get(&self, name: &str) -> Option<&ModelInfo> {
        self.models.get(name)
    }

    pub fn models(&self) -> impl Iterator<Item = &ModelInfo> {
        self.models.values()
    }
}

impl Default for ModelGraph {
    fn default() -> Self {
        Self::new()
    }
}
