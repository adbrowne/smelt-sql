use anyhow::Result;
use smelt_backend::Backend;
use smelt_core::config::Target;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Registry of backend instances, one per target.
///
/// Allows a single `smelt run` to route models to different backends
/// based on per-model target assignments.
pub struct BackendRegistry {
    backends: HashMap<String, Box<dyn Backend>>,
    targets: HashMap<String, Target>,
}

impl BackendRegistry {
    /// Create backends for all needed targets.
    ///
    /// Only targets in `needed` are instantiated. `default_target` is always included.
    pub async fn new(
        all_targets: &HashMap<String, Target>,
        needed: &HashSet<String>,
        project_dir: &Path,
        database_override: Option<PathBuf>,
    ) -> Result<Self> {
        let mut backends = HashMap::new();
        let mut targets = HashMap::new();

        for target_name in needed {
            let target_config = all_targets.get(target_name).ok_or_else(|| {
                let available: Vec<_> = all_targets.keys().cloned().collect();
                anyhow::anyhow!(
                    "Target '{}' not found in smelt.yml. Available targets: {}",
                    target_name,
                    available.join(", ")
                )
            })?;

            let backend = create_backend(
                target_name,
                target_config,
                project_dir,
                database_override.clone(),
            )
            .await?;

            backends.insert(target_name.clone(), backend);
            targets.insert(target_name.clone(), target_config.clone());
        }

        Ok(Self { backends, targets })
    }

    /// Get the backend for a target name.
    pub fn get(&self, target_name: &str) -> &dyn Backend {
        self.backends[target_name].as_ref()
    }

    /// Get the target config for a target name.
    pub fn target_config(&self, target_name: &str) -> &Target {
        &self.targets[target_name]
    }

    /// Get the single backend (for backward-compatible single-target runs).
    ///
    /// Panics if the registry has more than one backend.
    pub fn single_backend(&self) -> &dyn Backend {
        assert_eq!(
            self.backends.len(),
            1,
            "single_backend() called with {} backends",
            self.backends.len()
        );
        self.backends
            .values()
            .next()
            .expect("assert above guarantees exactly one backend")
            .as_ref()
    }
}

async fn create_backend(
    target_name: &str,
    target_config: &Target,
    project_dir: &Path,
    database_override: Option<PathBuf>,
) -> Result<Box<dyn Backend>> {
    smelt_backends::create_backend(target_name, target_config, project_dir, database_override).await
}
