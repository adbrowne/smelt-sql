use std::collections::{BTreeSet, HashMap};

use crate::types::PartitionGrainConfig;
pub use smelt_core::config::TimeseriesConfig;

/// Information about a single model for the optimizer.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ModelInfo {
    pub name: String,
    /// Raw SQL content (including any frontmatter).
    pub sql: String,
    /// Model names this model references via smelt.ref().
    pub refs: Vec<String>,
    /// Time-dimension declaration (event_time_column, partition_column, granularity).
    pub timeseries_config: Option<TimeseriesConfig>,
    /// Incremental configuration parsed from frontmatter, if any.
    pub incremental_config: Option<PartitionGrainConfig>,
    /// Columns declared `columns.<c>.contract: plausible` in the model's
    /// `.sql` frontmatter (`docs/specs/models.md` §"`columns:` — column
    /// metadata") — the sole surviving surface for the non-determinism
    /// payload opt-in the retired `batched.nondeterministic_columns` list
    /// form used to carry. Consumed by `check_nondeterminism`.
    pub plausible_columns: BTreeSet<String>,
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
