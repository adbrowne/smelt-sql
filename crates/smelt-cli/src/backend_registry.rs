use anyhow::{Context, Result};
use smelt_backend::Backend;
use smelt_core::config::{BackendType, Target};
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

/// Create a single backend instance for a target.
#[allow(unreachable_code, unused_variables)]
async fn create_backend(
    target_name: &str,
    target_config: &Target,
    project_dir: &Path,
    database_override: Option<PathBuf>,
) -> Result<Box<dyn Backend>> {
    match target_config.backend_type() {
        BackendType::DuckDB => {
            #[cfg(feature = "duckdb")]
            {
                use smelt_backend_duckdb::DuckDbBackend;

                let database = target_config
                    .database
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("DuckDB target requires 'database' field"))?;

                let db_path = database_override.unwrap_or_else(|| project_dir.join(database));
                tracing::info!("Backend [{}]: DuckDB", target_name);
                tracing::info!("Database: {}", db_path.display());

                Ok(Box::new(
                    DuckDbBackend::new(&db_path, &target_config.schema)
                        .await
                        .with_context(|| format!("Failed to initialize DuckDB at {:?}", db_path))?,
                ))
            }
            #[cfg(not(feature = "duckdb"))]
            {
                Err(anyhow::anyhow!(
                    "DuckDB backend not available. Rebuild with --features duckdb"
                ))
            }
        }
        BackendType::Spark => {
            #[cfg(feature = "spark")]
            {
                use smelt_backend_spark::SparkBackend;

                let connect_url = target_config
                    .connect_url
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("Spark target requires 'connect_url' field"))?;

                let default_catalog = "spark_catalog".to_string();
                let catalog = target_config.catalog.as_ref().unwrap_or(&default_catalog);

                tracing::info!("Backend [{}]: Spark", target_name);
                tracing::info!("Connect URL: {}", connect_url);
                tracing::info!("Catalog: {}", catalog);

                Ok(Box::new(
                    SparkBackend::new(
                        connect_url,
                        catalog,
                        &target_config.schema,
                        target_config.warehouse.as_deref(),
                    )
                    .await
                    .with_context(|| format!("Failed to connect to Spark at {}", connect_url))?,
                ))
            }
            #[cfg(not(feature = "spark"))]
            {
                Err(anyhow::anyhow!(
                    "Spark backend not available. Rebuild with --features spark"
                ))
            }
        }
    }
}
