use std::path::{Path, PathBuf};

use anyhow::Result;
use smelt_backend::Backend;
use smelt_core::config::{BackendType, Target};

/// Create a single backend instance for a target.
///
/// This is the single canonical place where `target_type → Box<dyn Backend>`
/// selection lives. Both `smelt-cli` and `smelt-ui` delegate their
/// `BackendFactory::create` here.
///
/// `database_override` is a CLI-level opt (mirrors `smelt run --database`);
/// UI callers pass `None`.
#[allow(unreachable_code, unused_variables)]
pub async fn create_backend(
    target_name: &str,
    target_config: &Target,
    project_dir: &Path,
    database_override: Option<PathBuf>,
) -> Result<Box<dyn Backend>> {
    match target_config.backend_type()? {
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
                    DuckDbBackend::new_with_settings(
                        &db_path,
                        &target_config.schema,
                        target_config.settings.as_ref(),
                    )
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
