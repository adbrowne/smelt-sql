use anyhow::{Context, Result};
use chrono::Utc;
use smelt_backend::{Backend, SqlDialect};
use smelt_core::config::TableFormat;
use smelt_core::metadata::{ModelMetadata, SchemaEvolutionStrategy};
use smelt_dialect::BackendCapabilities;
use smelt_logical::maintenance::emit::{MaintenanceStatement, StatementGroup};
use smelt_state::ddl_spark::SparkTableFormat;
use smelt_state::file_store::FileStore;
use smelt_state::intervals::compute_model_hash;
use smelt_state::schema_tracking::{
    diff_schemas, plan_migration_for_backend, DdlBackend, DeployedColumn, DeployedSchema,
    MigrationAction,
};
use std::collections::HashMap;

/// Infer the deployed schema columns for a model by running Salsa type
/// inference.
///
/// Returns an empty vec if the workspace is not initialised or the file is not
/// registered — callers treat an empty result as "skip schema evolution".
pub fn infer_deployed_columns(
    db: &smelt_db::Database,
    model: &smelt_core::ModelFile,
) -> Vec<DeployedColumn> {
    let ws = match smelt_db::Workspace::try_get(db) {
        Some(ws) => ws,
        None => return vec![],
    };
    let file = match db.source_file(&model.path) {
        Some(f) => f,
        None => return vec![],
    };
    let schema = smelt_db::typed_model_schema(db, ws, file);
    schema
        .columns
        .iter()
        .filter(|c| c.name != "*")
        .map(|c| {
            let (data_type, nullable) = match &c.data_type {
                Some(tc) => (tc.data_type.to_sql(), tc.nullable),
                None => ("UNKNOWN".to_string(), true),
            };
            DeployedColumn {
                name: c.name.clone(),
                data_type,
                nullable,
            }
        })
        .collect()
}

/// Extract column default values and backfill expressions from model metadata.
///
/// Returns `(column_defaults, backfill_exprs)` where:
/// - `column_defaults` maps column name → SQL expression string (from `default:` in frontmatter)
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
                        .map(|expr| (name.clone(), expr.clone()))
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
    Migrated {
        statements: Vec<String>,
        /// Names of newly-added columns whose backfill `UPDATE` was folded
        /// into this SAME `StatementGroup` as the `ALTER TABLE ... ADD
        /// COLUMN` (declared `backfill:`/`default:` directives, or a
        /// derived `Technique::InPlaceUpdate` assignment the caller merged
        /// into `backfill_exprs` before calling `check_and_migrate` —
        /// `docs/plans/20260809-sensitivity-precision.md` Phase 6). Callers
        /// that separately dispatch an `InPlaceUpdate` cell's backfill use
        /// this to skip columns already backfilled atomically here.
        backfilled_columns: Vec<String>,
    },
    /// Full refresh required due to destructive changes.
    FullRefreshRequired { reason: String },
    /// Column removal blocked — requires --allow-column-removal flag.
    ColumnRemovalBlocked { columns: Vec<String> },
    /// Full refresh required but `--allow-full-refresh` not set — blocked.
    FullRefreshBlocked { reason: String },
    /// Table rewrite performed (Spark: CREATE TABLE tmp AS SELECT ... FROM original).
    TableRewrite { description: String },
}

/// Construct a `DdlBackend` from a SQL dialect and optional table format.
///
/// For DuckDB, returns `DdlBackend::DuckDb`.
/// For Spark, selects Delta or Parquet capabilities based on the table format.
/// The `catalog` defaults to `"spark_catalog"` — callers should provide the actual
/// catalog name if available.
pub fn ddl_backend_for_dialect(
    dialect: SqlDialect,
    table_format: Option<TableFormat>,
    catalog: Option<&str>,
) -> DdlBackend {
    match dialect {
        SqlDialect::DuckDB | SqlDialect::PostgreSQL => DdlBackend::DuckDb,
        SqlDialect::BigQuery => DdlBackend::BigQuery,
        SqlDialect::SparkSQL => {
            let format = match table_format {
                Some(TableFormat::Parquet) => SparkTableFormat::Parquet,
                Some(TableFormat::Delta) | None => SparkTableFormat::Delta,
            };
            let capabilities = match format {
                SparkTableFormat::Delta => BackendCapabilities::spark_delta(),
                SparkTableFormat::Parquet => BackendCapabilities::spark_parquet(),
            };
            DdlBackend::Spark {
                catalog: catalog.unwrap_or("spark_catalog").to_string(),
                format,
                capabilities,
            }
        }
    }
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

/// Whether a `schema_evolution: strategy: full_refresh` model must rebuild
/// under its new definition this run.
///
/// A model that opts out of `ALTER`-based evolution has no migration path of
/// its own; per `docs/specs/definition_deltas.md` §"The atomicity rule", it
/// rebuilds instead of falling through to a non-atomic standalone backfill.
/// Only relevant when the deployed and inferred schemas actually diverge —
/// an unchanged model still runs incrementally.
pub fn full_refresh_escape_requires_rebuild(
    strategy: Option<&SchemaEvolutionStrategy>,
    deployed: &[DeployedColumn],
    inferred: &[DeployedColumn],
) -> bool {
    if !matches!(strategy, Some(SchemaEvolutionStrategy::FullRefresh)) {
        return false;
    }
    !diff_schemas(deployed, inferred).is_empty()
}

/// Extract the column name a single-column `ADD COLUMN <name> ...` statement
/// targets, or `None` for anything else (a multi-column `ADD COLUMNS (...)`
/// form, a nested dot-path `ADD COLUMN parent.child ...`, or a non-`ADD
/// COLUMN` statement such as the backfill `UPDATE`).
fn single_add_column_name(statement: &str) -> Option<String> {
    let upper = statement.to_uppercase();
    let idx = upper.find("ADD COLUMN")?;
    // Reject the plural "ADD COLUMNS (...)" multi-column form — not safe to
    // reconcile column-by-column here.
    if upper[idx..].starts_with("ADD COLUMNS") {
        return None;
    }
    let rest = statement[idx + "ADD COLUMN".len()..].trim_start();
    let token = rest.split_whitespace().next()?;
    let name = token.trim_matches(|c| c == '"' || c == '`');
    if name.contains('.') {
        // A nested dot-path targets a sub-field of an already-present
        // top-level column — never reconcile it away by the parent's name.
        return None;
    }
    Some(name.to_string())
}

/// Reconcile a migration group's derived statements against the target's
/// *physical* columns, dropping an `ADD COLUMN` whose column is already
/// present while keeping its backfill `UPDATE`.
///
/// Repairs a partially-applied migration on a backend without transactional
/// DDL: the `ADD COLUMN` committed, the group's own backfill statement (or a
/// later one) failed, and a naive retry would re-emit the same `ADD COLUMN`
/// against a column that now already exists. Identity fn on the ordinary
/// (nothing partially applied) path — see
/// `docs/specs/definition_deltas.md` §"The atomicity rule".
pub fn reconcile_add_columns(statements: &[String], physical_columns: &[String]) -> Vec<String> {
    statements
        .iter()
        .filter(|stmt| match single_add_column_name(stmt) {
            Some(name) => !physical_columns
                .iter()
                .any(|c| c.eq_ignore_ascii_case(&name)),
            None => true,
        })
        .cloned()
        .collect()
}

/// Check for schema evolution and apply migrations if needed.
///
/// `ddl_backend` selects the DDL generator (DuckDB vs Spark+Delta/Parquet).
/// When `None`, defaults to DuckDB.
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
    allow_full_refresh: bool,
    dry_run: bool,
    column_defaults: &HashMap<String, String>,
    backfill_exprs: &HashMap<String, String>,
    ddl_backend: Option<&DdlBackend>,
    retry: &crate::execute::RetryPolicy<'_>,
) -> Result<SchemaEvolutionResult> {
    let model_hash = compute_model_hash(model_sql);
    let default_backend = DdlBackend::DuckDb;
    let ddl_backend = ddl_backend.unwrap_or(&default_backend);

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

    // Plan the migration using the appropriate backend DDL generator
    let action = plan_migration_for_backend(
        schema,
        model_name,
        &diff,
        allow_column_removal,
        column_defaults,
        backfill_exprs,
        ddl_backend,
        &deployed_schema.columns,
        inferred_columns,
    );

    match action {
        MigrationAction::NoChange => Ok(SchemaEvolutionResult::NoChange),

        MigrationAction::AlterTable { statements } => {
            // Columns this migration's ADD COLUMN will backfill in the
            // same group, computed from the diff (not the raw statement
            // text) so it stays correct regardless of DDL phrasing.
            let backfilled_columns: Vec<String> = diff
                .changes
                .iter()
                .filter_map(|c| match c {
                    smelt_state::schema_tracking::SchemaChange::AddColumn { name, .. }
                        if backfill_exprs.contains_key(name.as_str()) =>
                    {
                        Some(name.clone())
                    }
                    _ => None,
                })
                .collect();

            if dry_run {
                return Ok(SchemaEvolutionResult::Migrated {
                    statements,
                    backfilled_columns,
                });
            }

            // Reconcile the derived statements against the target's
            // physical columns before executing — repairs a migration
            // group that partially applied on a backend without
            // transactional DDL (docs/specs/definition_deltas.md §"The
            // atomicity rule"). `information_schema.columns` is read rather
            // than a `SELECT * ... LIMIT 0` projection because a zero-row
            // result set legitimately produces zero Arrow record batches
            // (no batch means no schema to read), while an
            // `information_schema` row exists per column regardless of the
            // table's own row count.
            let probe_sql = format!(
                "SELECT column_name FROM information_schema.columns \
                 WHERE table_schema = '{schema}' AND table_name = '{model_name}'"
            );
            let probe_batches = backend
                .execute_sql(&probe_sql)
                .await
                .with_context(|| format!("Failed to read physical schema for {model_name}"))?;
            let mut physical_columns: Vec<String> = Vec::new();
            for batch in &probe_batches {
                use arrow::array::Array;
                let strings = batch
                    .column(0)
                    .as_any()
                    .downcast_ref::<arrow::array::StringArray>()
                    .with_context(|| {
                        format!("information_schema.columns probe for {model_name} returned a non-string column_name")
                    })?;
                physical_columns.extend((0..strings.len()).map(|i| strings.value(i).to_string()));
            }
            let statements = reconcile_add_columns(&statements, &physical_columns);

            // Execute ALTER TABLE statements
            let use_transaction = backend.capabilities().supports_transactional_ddl;

            // Run every ALTER TABLE statement as one `StatementGroup` via
            // `Backend::execute_statement_group` rather than issuing
            // "BEGIN TRANSACTION" / each statement / "COMMIT"/"ROLLBACK" as
            // separate `execute_sql` calls. Each `execute_sql` call only
            // holds the backend's connection lock for its own duration
            // (`crates/smelt-backend-duckdb/CLAUDE.md` — one
            // `spawn_blocking` per call); under DAG-parallel model
            // execution (`--jobs`), a concurrently-running model's
            // statements could interleave into the gaps between this
            // model's BEGIN/ALTER/COMMIT, corrupting the shared DuckDB
            // connection's transaction state ("cannot start a transaction
            // within a transaction" / "current transaction is aborted").
            // `execute_statement_group` is the same single choke point
            // every other maintenance statement group already routes
            // through, and the DuckDB backend's override holds the
            // connection mutex for the statement group's full duration.
            let group = StatementGroup {
                statements: statements
                    .iter()
                    .map(|s| MaintenanceStatement { sql: s.clone() })
                    .collect(),
                transactional: use_transaction,
            };
            if let Err(e) = crate::execute::retry_backend_call(retry, || {
                backend.execute_statement_group(&group)
            })
            .await
            {
                return Err(anyhow::anyhow!("Schema migration failed: {}", e));
            }

            // Save updated schema
            let new_schema = DeployedSchema {
                model: model_name.to_string(),
                version: deployed_schema.version + 1,
                deployed_at: Utc::now(),
                model_hash,
                model_sql: Some(model_sql.to_string()),
                columns: inferred_columns.to_vec(),
            };
            file_store
                .save_schema(&new_schema)
                .with_context(|| format!("Failed to save updated schema for {}", model_name))?;

            Ok(SchemaEvolutionResult::Migrated {
                statements: statements.clone(),
                backfilled_columns,
            })
        }

        MigrationAction::FullRefresh { reason } => {
            Ok(SchemaEvolutionResult::FullRefreshRequired { reason })
        }

        MigrationAction::RequiresColumnRemovalFlag { columns } => {
            Ok(SchemaEvolutionResult::ColumnRemovalBlocked { columns })
        }

        MigrationAction::FullRefreshBlocked { reason } => {
            if allow_full_refresh {
                // User opted in — treat as a full refresh
                Ok(SchemaEvolutionResult::FullRefreshRequired { reason })
            } else {
                Ok(SchemaEvolutionResult::FullRefreshBlocked { reason })
            }
        }

        MigrationAction::TableRewrite { select_expr } => {
            if allow_full_refresh {
                // Table rewrite allowed — report it back for execution
                Ok(SchemaEvolutionResult::TableRewrite {
                    description: select_expr,
                })
            } else {
                // Table rewrite blocked without --allow-full-refresh
                Ok(SchemaEvolutionResult::FullRefreshBlocked {
                    reason: format!(
                        "Schema change requires table rewrite. Use --allow-full-refresh to permit. Details: {}",
                        select_expr
                    ),
                })
            }
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
        model_sql: Some(model_sql.to_string()),
        columns: columns.to_vec(),
    };
    file_store
        .save_schema(&schema)
        .with_context(|| format!("Failed to save schema for {}", model_name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use smelt_state::ddl_spark::SparkTableFormat;
    use smelt_state::schema_tracking::DdlBackend;

    #[test]
    fn reconcile_add_columns_skips_physically_present_column() {
        let statements = vec![
            "ALTER TABLE main.t ADD COLUMN val_doubled BIGINT".to_string(),
            "UPDATE main.t SET val_doubled = val * 2".to_string(),
        ];
        let physical_columns = vec![
            "event_date".to_string(),
            "val".to_string(),
            "val_doubled".to_string(),
        ];
        let reconciled = reconcile_add_columns(&statements, &physical_columns);
        assert_eq!(
            reconciled,
            vec!["UPDATE main.t SET val_doubled = val * 2".to_string()],
            "the ADD COLUMN for an already-present column must be dropped, the backfill kept"
        );
    }

    #[test]
    fn reconcile_add_columns_is_identity_when_nothing_present() {
        let statements = vec![
            "ALTER TABLE main.t ADD COLUMN val_doubled BIGINT".to_string(),
            "UPDATE main.t SET val_doubled = val * 2".to_string(),
        ];
        let physical_columns = vec!["event_date".to_string(), "val".to_string()];
        let reconciled = reconcile_add_columns(&statements, &physical_columns);
        assert_eq!(reconciled, statements);
    }

    #[test]
    fn full_refresh_escape_forces_rebuild_only_when_schema_changed() {
        let deployed = vec![DeployedColumn {
            name: "val".to_string(),
            data_type: "BIGINT".to_string(),
            nullable: false,
        }];
        let inferred_unchanged = deployed.clone();
        assert!(!full_refresh_escape_requires_rebuild(
            Some(&SchemaEvolutionStrategy::FullRefresh),
            &deployed,
            &inferred_unchanged,
        ));

        let inferred_changed = vec![
            deployed[0].clone(),
            DeployedColumn {
                name: "val_doubled".to_string(),
                data_type: "BIGINT".to_string(),
                nullable: true,
            },
        ];
        assert!(full_refresh_escape_requires_rebuild(
            Some(&SchemaEvolutionStrategy::FullRefresh),
            &deployed,
            &inferred_changed,
        ));

        // A non-`FullRefresh` strategy never triggers this escape, even
        // with a schema change.
        assert!(!full_refresh_escape_requires_rebuild(
            None,
            &deployed,
            &inferred_changed,
        ));
    }

    #[test]
    fn test_ddl_backend_duckdb() {
        let backend = ddl_backend_for_dialect(SqlDialect::DuckDB, None, None);
        assert!(matches!(backend, DdlBackend::DuckDb));
    }

    #[test]
    fn test_ddl_backend_duckdb_ignores_format() {
        // DuckDB ignores table format
        let backend = ddl_backend_for_dialect(SqlDialect::DuckDB, Some(TableFormat::Delta), None);
        assert!(matches!(backend, DdlBackend::DuckDb));
    }

    #[test]
    fn test_ddl_backend_postgresql_uses_duckdb() {
        // PostgreSQL uses DuckDB DDL generator (same SQL dialect)
        let backend = ddl_backend_for_dialect(SqlDialect::PostgreSQL, None, None);
        assert!(matches!(backend, DdlBackend::DuckDb));
    }

    #[test]
    fn test_ddl_backend_spark_defaults_to_delta() {
        let backend = ddl_backend_for_dialect(SqlDialect::SparkSQL, None, None);
        match backend {
            DdlBackend::Spark {
                format,
                capabilities,
                ..
            } => {
                assert_eq!(format, SparkTableFormat::Delta);
                assert!(capabilities.supports_column_mapping);
                assert!(capabilities.supports_merge_schema_write);
            }
            _ => panic!("Expected Spark backend"),
        }
    }

    #[test]
    fn test_ddl_backend_spark_delta() {
        let backend = ddl_backend_for_dialect(SqlDialect::SparkSQL, Some(TableFormat::Delta), None);
        match backend {
            DdlBackend::Spark {
                format,
                capabilities,
                catalog,
                ..
            } => {
                assert_eq!(format, SparkTableFormat::Delta);
                assert!(capabilities.supports_column_mapping);
                assert_eq!(catalog, "spark_catalog");
            }
            _ => panic!("Expected Spark backend"),
        }
    }

    #[test]
    fn test_ddl_backend_spark_parquet() {
        let backend =
            ddl_backend_for_dialect(SqlDialect::SparkSQL, Some(TableFormat::Parquet), None);
        match backend {
            DdlBackend::Spark {
                format,
                capabilities,
                ..
            } => {
                assert_eq!(format, SparkTableFormat::Parquet);
                assert!(!capabilities.supports_column_mapping);
            }
            _ => panic!("Expected Spark backend"),
        }
    }

    #[test]
    fn test_ddl_backend_spark_custom_catalog() {
        let backend = ddl_backend_for_dialect(
            SqlDialect::SparkSQL,
            Some(TableFormat::Delta),
            Some("unity_catalog"),
        );
        match backend {
            DdlBackend::Spark { catalog, .. } => {
                assert_eq!(catalog, "unity_catalog");
            }
            _ => panic!("Expected Spark backend"),
        }
    }

    #[test]
    fn save_deployed_schema_records_the_definition_sql() {
        let dir = tempfile::TempDir::new().unwrap();
        let file_store = FileStore::new(dir.path(), "dev");
        let sql = "SELECT id, amount - discount AS net_amount FROM orders";

        save_deployed_schema(
            &file_store,
            "net_orders",
            sql,
            &[DeployedColumn {
                name: "id".to_string(),
                data_type: "INTEGER".to_string(),
                nullable: false,
            }],
            None,
        )
        .expect("save_deployed_schema");

        let loaded = file_store
            .load_schema("net_orders")
            .expect("load_schema")
            .expect("schema present");
        assert_eq!(loaded.model_sql, Some(sql.to_string()));
    }
}
