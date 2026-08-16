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
                use smelt_core::config::TableFormat;

                let connect_url = target_config
                    .connect_url
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("Spark target requires 'connect_url' field"))?;

                let default_catalog = "spark_catalog".to_string();
                let catalog = target_config.catalog.as_ref().unwrap_or(&default_catalog);

                tracing::info!("Backend [{}]: Spark", target_name);
                tracing::info!("Connect URL: {}", redact_connect_url(connect_url));
                tracing::info!("Catalog: {}", catalog);

                let use_delta = target_config
                    .table_format()
                    .map(|f| matches!(f, TableFormat::Delta))
                    .unwrap_or(true);

                Ok(Box::new(
                    SparkBackend::new(
                        connect_url,
                        catalog,
                        &target_config.schema,
                        target_config.warehouse.as_deref(),
                        use_delta,
                    )
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "Failed to connect to Spark at {}: {}",
                            redact_connect_url(connect_url),
                            e
                        )
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
        BackendType::BigQuery => {
            #[cfg(feature = "bigquery")]
            {
                use smelt_backend_bigquery::BigQueryBackend;

                let project = target_config
                    .project
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("BigQuery target requires 'project' field"))?;
                let dataset = target_config
                    .dataset
                    .as_ref()
                    .unwrap_or(&target_config.schema);

                // Read the token here rather than in the adapter so a missing
                // one is a smelt-level diagnostic naming the script that mints
                // it, not a Python traceback. There is deliberately no
                // application-default-credentials fallback
                // (`multi_backend.md` §Surface).
                let access_token = std::env::var("SMELT_BQ_ACCESS_TOKEN").map_err(|_| {
                    anyhow::anyhow!(
                        "BigQuery target requires SMELT_BQ_ACCESS_TOKEN. \
                         Mint one with `bash scripts/bigquery-auth.sh` then \
                         `source scripts/bigquery-env.sh`. smelt never falls back to \
                         Google application-default credentials."
                    )
                })?;

                tracing::info!("Backend [{}]: BigQuery", target_name);
                tracing::info!("Project: {}, dataset: {}", project, dataset);

                Ok(Box::new(
                    BigQueryBackend::new(
                        project,
                        dataset,
                        target_config.location.as_deref(),
                        &access_token,
                    )
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!("Failed to connect to BigQuery project {}: {}", project, e)
                    })?,
                ))
            }
            #[cfg(not(feature = "bigquery"))]
            {
                Err(anyhow::anyhow!(
                    "BigQuery backend not available. Rebuild with --features bigquery"
                ))
            }
        }
    }
}

/// Strip Spark Connect URL parameters (`;token=...;use_ssl=...`) before a
/// `connect_url` reaches a log line or error message. Params start at the
/// first `;` per the Spark Connect URI grammar (`sc://host:port/;k=v;...`) —
/// this is where an interpolated `${DATABRICKS_TOKEN}` etc. lands (spec:
/// `multi_backend.md` §"Connection security" — "smelt never...logs the
/// resolved URL").
#[cfg(feature = "spark")]
fn redact_connect_url(connect_url: &str) -> &str {
    connect_url
        .split_once(';')
        .map(|(host_part, _)| host_part)
        .unwrap_or(connect_url)
}

#[cfg(all(test, feature = "spark"))]
mod tests {
    use super::redact_connect_url;

    #[test]
    fn redact_connect_url_strips_token_and_tls_params() {
        assert_eq!(
            redact_connect_url("sc://host:443/;token=secret;use_ssl=true"),
            "sc://host:443/"
        );
    }

    #[test]
    fn redact_connect_url_passes_through_url_with_no_params() {
        assert_eq!(
            redact_connect_url("sc://localhost:15002"),
            "sc://localhost:15002"
        );
    }
}
