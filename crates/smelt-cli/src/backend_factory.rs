//! CLI `BackendFactory` implementation.
//!
//! Wraps the CLI's `BackendRegistry` creation logic into the
//! `smelt_runtime::BackendFactory` trait so `commands/run.rs` can delegate
//! backend creation to `execute_project` rather than building backends itself.

use std::path::{Path, PathBuf};

use anyhow::Result;
use smelt_backend::Backend;
use smelt_core::config::{BackendType, Target};
use smelt_runtime::execute::{BackendFactory, BackendFuture};

/// A `BackendFactory` that constructs the same backends the CLI's
/// `BackendRegistry` previously built: DuckDB and (optionally) Spark.
///
/// An optional `database_override` lets `smelt run --database <path>` redirect
/// DuckDB output to a different file without touching the config.
pub struct CliBackendFactory {
    pub database_override: Option<PathBuf>,
}

impl BackendFactory for CliBackendFactory {
    fn create<'a>(
        &'a self,
        target_name: &'a str,
        target_config: &'a Target,
        project_dir: &'a Path,
    ) -> BackendFuture<'a> {
        let database_override = self.database_override.clone();
        Box::pin(async move {
            create_backend(target_name, target_config, project_dir, database_override).await
        })
    }
}

/// Create a single backend instance for a target. Mirrors the `create_backend`
/// function in `backend_registry.rs` — kept in sync by construction (both
/// delegate to the same backend crates).
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
                        .map_err(|e| {
                            anyhow::anyhow!("Failed to initialize DuckDB at {:?}: {}", db_path, e)
                        })?,
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
                    .map_err(|e| {
                        anyhow::anyhow!("Failed to connect to Spark at {}: {}", connect_url, e)
                    })?,
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
