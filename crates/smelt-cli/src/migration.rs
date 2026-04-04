use anyhow::{Context, Result};
use chrono::Utc;
use smelt_backend::Backend;
use smelt_core::metadata::{yaml_value_to_sql_literal, ModelMetadata};
use smelt_state::file_store::FileStore;
use smelt_state::intervals::compute_model_hash;
use smelt_state::schema_tracking::{
    diff_schemas, plan_migration, DeployedColumn, DeployedSchema, MigrationAction,
};
use std::collections::HashMap;
use tracing::warn;

/// Extract column default values and backfill expressions from model metadata.
///
/// Returns `(column_defaults, backfill_exprs)` where:
/// - `column_defaults` maps column name → SQL literal (from `default:` in frontmatter)
/// - `backfill_exprs` maps column name → SQL expression (from `backfill:` in frontmatter)
pub fn extract_evolution_maps(
    metadata: Option<&ModelMetadata>,
) -> (HashMap<String, String>, HashMap<String, String>) {
    metadata
        .map(|m| {
            let defaults: HashMap<String, String> = m
                .columns
                .iter()
                .filter_map(|(name, col_meta)| {
                    col_meta
                        .default
                        .as_ref()
                        .and_then(|v| match yaml_value_to_sql_literal(v) {
                            Ok(sql) => Some((name.clone(), sql)),
                            Err(e) => {
                                warn!("Column '{}': {} — ignoring default", name, e);
                                None
                            }
                        })
                })
                .collect();
            let backfills: HashMap<String, String> = m
                .columns
                .iter()
                .filter_map(|(name, col_meta)| {
                    col_meta
                        .backfill
                        .as_ref()
                        .map(|expr| (name.clone(), expr.clone()))
                })
                .collect();
            (defaults, backfills)
        })
        .unwrap_or_default()
}

/// Result of checking schema evolution for a model.
#[derive(Debug)]
pub enum SchemaEvolutionResult {
    /// First deployment — no prior schema exists.
    FirstDeployment,
    /// No schema changes detected.
    NoChange,
    /// ALTER TABLE statements were executed successfully.
    Migrated { statements: Vec<String> },
    /// Full refresh required due to destructive changes.
    FullRefreshRequired { reason: String },
    /// Column removal blocked — requires --allow-column-removal flag.
    ColumnRemovalBlocked { columns: Vec<String> },
    /// Full refresh required but `--allow-full-refresh` not set — blocked.
    FullRefreshBlocked { reason: String },
    /// Table rewrite performed (Spark: CREATE TABLE tmp AS SELECT ... FROM original).
    TableRewrite { description: String },
}

/// Infer the current schema columns from the model's inferred types.
///
/// This converts smelt-db Column data into DeployedColumn format for comparison.
pub fn columns_from_inferred(columns: &[(String, Option<String>, bool)]) -> Vec<DeployedColumn> {
    columns
        .iter()
        .map(|(name, data_type, nullable)| DeployedColumn {
            name: name.clone(),
            data_type: data_type.clone().unwrap_or_else(|| "UNKNOWN".to_string()),
            nullable: *nullable,
        })
        .collect()
}

/// Check for schema evolution and apply migrations if needed.
///
/// Returns what action was taken (or what action is required).
#[allow(clippy::too_many_arguments)]
pub async fn check_and_migrate(
    backend: &dyn Backend,
    file_store: &FileStore,
    model_name: &str,
    model_sql: &str,
    schema: &str,
    inferred_columns: &[DeployedColumn],
    allow_column_removal: bool,
    _allow_full_refresh: bool,
    dry_run: bool,
    column_defaults: &HashMap<String, String>,
    backfill_exprs: &HashMap<String, String>,
) -> Result<SchemaEvolutionResult> {
    let model_hash = compute_model_hash(model_sql);

    // Load deployed schema
    let deployed = file_store
        .load_schema(model_name)
        .with_context(|| format!("Failed to load deployed schema for {}", model_name))?;

    let deployed_schema = match deployed {
        None => {
            // First deployment — save the schema after execution
            return Ok(SchemaEvolutionResult::FirstDeployment);
        }
        Some(s) => s,
    };

    // Compare schemas
    let diff = diff_schemas(&deployed_schema.columns, inferred_columns);

    if diff.is_empty() {
        return Ok(SchemaEvolutionResult::NoChange);
    }

    // Plan the migration
    let action = plan_migration(
        schema,
        model_name,
        &diff,
        allow_column_removal,
        column_defaults,
        backfill_exprs,
    );

    match action {
        MigrationAction::NoChange => Ok(SchemaEvolutionResult::NoChange),

        MigrationAction::AlterTable { statements } => {
            if dry_run {
                return Ok(SchemaEvolutionResult::Migrated { statements });
            }

            // Execute ALTER TABLE statements
            let use_transaction = backend.capabilities().supports_transactional_ddl;

            if use_transaction {
                backend
                    .execute_sql("BEGIN TRANSACTION")
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to begin transaction: {}", e))?;
            }

            for stmt in &statements {
                if let Err(e) = backend.execute_sql(stmt).await {
                    if use_transaction {
                        let _ = backend.execute_sql("ROLLBACK").await;
                    }
                    return Err(anyhow::anyhow!(
                        "Schema migration failed on '{}': {}",
                        stmt,
                        e
                    ));
                }
            }

            if use_transaction {
                backend
                    .execute_sql("COMMIT")
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to commit transaction: {}", e))?;
            }

            // Save updated schema
            let new_schema = DeployedSchema {
                model: model_name.to_string(),
                version: deployed_schema.version + 1,
                deployed_at: Utc::now(),
                model_hash,
                columns: inferred_columns.to_vec(),
            };
            file_store
                .save_schema(&new_schema)
                .with_context(|| format!("Failed to save updated schema for {}", model_name))?;

            Ok(SchemaEvolutionResult::Migrated {
                statements: statements.clone(),
            })
        }

        MigrationAction::FullRefresh { reason } => {
            Ok(SchemaEvolutionResult::FullRefreshRequired { reason })
        }

        MigrationAction::RequiresColumnRemovalFlag { columns } => {
            Ok(SchemaEvolutionResult::ColumnRemovalBlocked { columns })
        }

        MigrationAction::FullRefreshBlocked { reason } => {
            if _allow_full_refresh {
                // User opted in — treat as a full refresh
                Ok(SchemaEvolutionResult::FullRefreshRequired { reason })
            } else {
                Ok(SchemaEvolutionResult::FullRefreshBlocked { reason })
            }
        }

        MigrationAction::TableRewrite { select_expr } => {
            // Table rewrite is a Spark-specific operation (Phase 10 will implement execution).
            // For now, report it back to the caller.
            Ok(SchemaEvolutionResult::TableRewrite {
                description: select_expr,
            })
        }
    }
}

/// Save the deployed schema after a successful model execution.
///
/// Called after first deployment or after full refresh.
pub fn save_deployed_schema(
    file_store: &FileStore,
    model_name: &str,
    model_sql: &str,
    columns: &[DeployedColumn],
    existing_version: Option<u32>,
) -> Result<()> {
    let model_hash = compute_model_hash(model_sql);
    let schema = DeployedSchema {
        model: model_name.to_string(),
        version: existing_version.map_or(1, |v| v + 1),
        deployed_at: Utc::now(),
        model_hash,
        columns: columns.to_vec(),
    };
    file_store
        .save_schema(&schema)
        .with_context(|| format!("Failed to save schema for {}", model_name))
}
