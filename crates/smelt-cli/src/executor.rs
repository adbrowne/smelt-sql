use crate::errors::CliError;
use anyhow::Result;
use smelt_backend::{
    Backend, ExecutionResult, IncrementalStrategy, Materialization, MaterializationStrategy,
    PartitionRange,
};
use smelt_core::SourcesConfig;
use smelt_runtime::CompiledModel;

/// Execute a compiled model using any Backend implementation.
pub async fn execute_model(
    backend: &dyn Backend,
    compiled: &CompiledModel,
    schema: &str,
    show_results: bool,
) -> Result<ExecutionResult> {
    // Convert CLI Materialization to Backend Materialization
    let materialization = match compiled.materialization {
        crate::config::Materialization::Table => Materialization::Table,
        crate::config::Materialization::View => Materialization::View,
        crate::config::Materialization::Ephemeral => {
            unreachable!("Ephemeral models should be inlined as CTEs, not executed directly")
        }
    };

    backend
        .execute_model(
            schema,
            &compiled.name,
            &compiled.sql,
            materialization,
            show_results,
        )
        .await
        .map_err(|e| {
            CliError::ExecutionError {
                model: compiled.name.clone(),
                sql: compiled.sql.clone(),
                source: e.into(),
            }
            .into()
        })
}

/// Execute a compiled model incrementally using the resolved strategy.
///
/// This function:
/// 1. Applies the strategy (DELETE+INSERT, MERGE, APPEND, or INSERT OVERWRITE)
/// 2. Auto-creates the table on first run if it doesn't exist
pub async fn execute_model_incremental(
    backend: &dyn Backend,
    compiled: &CompiledModel,
    schema: &str,
    partition: PartitionRange,
    inc_strategy: IncrementalStrategy,
    unique_key: Vec<String>,
    show_results: bool,
) -> Result<ExecutionResult> {
    // Views can't be incremental - warn and use full refresh
    if matches!(
        compiled.materialization,
        crate::config::Materialization::View
    ) {
        tracing::warn!(
            "{} is a view, using full refresh (views cannot be incremental)",
            compiled.name
        );
        return execute_model(backend, compiled, schema, show_results).await;
    }

    let strategy = MaterializationStrategy::Incremental {
        partition,
        strategy: inc_strategy,
        unique_key,
    };

    backend
        .execute_model_incremental(
            schema,
            &compiled.name,
            &compiled.sql,
            Materialization::Table,
            strategy,
            show_results,
        )
        .await
        .map_err(|e| {
            CliError::ExecutionError {
                model: compiled.name.clone(),
                sql: compiled.sql.clone(),
                source: e.into(),
            }
            .into()
        })
}

/// Execute a multi-step plan (e.g., from cube split optimization).
///
/// Iterates through execution steps, creating temp tables, running the final
/// query to produce the model output, and cleaning up temp tables.
pub async fn execute_plan(
    backend: &dyn Backend,
    model_name: &str,
    steps: &[smelt_planner::ExecutionStep],
    schema: &str,
    show_results: bool,
) -> Result<ExecutionResult> {
    let start = std::time::Instant::now();

    for step in steps {
        match step {
            smelt_planner::ExecutionStep::CreateTemp { name, sql } => {
                let create_sql = format!("CREATE TEMP TABLE {} AS {}", name, sql);
                backend
                    .execute_sql(&create_sql)
                    .await
                    .map_err(|e| CliError::ExecutionError {
                        model: model_name.to_string(),
                        sql: create_sql.clone(),
                        source: e.into(),
                    })?;
            }
            smelt_planner::ExecutionStep::AppendToTemp { name, sql } => {
                let insert_sql = format!("INSERT INTO {} {}", name, sql);
                backend
                    .execute_sql(&insert_sql)
                    .await
                    .map_err(|e| CliError::ExecutionError {
                        model: model_name.to_string(),
                        sql: insert_sql.clone(),
                        source: e.into(),
                    })?;
            }
            smelt_planner::ExecutionStep::FinalQuery { sql } => {
                backend
                    .drop_table_if_exists(schema, model_name)
                    .await
                    .map_err(|e| CliError::ExecutionError {
                        model: model_name.to_string(),
                        sql: "DROP TABLE".to_string(),
                        source: e.into(),
                    })?;
                backend
                    .create_table_as(schema, model_name, sql)
                    .await
                    .map_err(|e| CliError::ExecutionError {
                        model: model_name.to_string(),
                        sql: sql.clone(),
                        source: e.into(),
                    })?;
            }
            smelt_planner::ExecutionStep::DropTemp { name } => {
                let drop_sql = format!("DROP TABLE IF EXISTS {}", name);
                // Best-effort cleanup — don't fail the whole plan if drop fails
                let _ = backend.execute_sql(&drop_sql).await;
            }
        }
    }

    let duration = start.elapsed();
    let row_count = backend.get_row_count(schema, model_name).await.unwrap_or(0);

    let preview = if show_results {
        backend.get_preview(schema, model_name, 10).await.ok()
    } else {
        None
    };

    Ok(ExecutionResult {
        model_name: model_name.to_string(),
        duration,
        row_count,
        preview,
    })
}

/// Execute a multi-step plan incrementally (cube split + incremental).
///
/// Applies time filtering to each step's SQL before execution, and uses
/// the resolved strategy for the final table update.
#[allow(clippy::too_many_arguments)]
pub async fn execute_plan_incremental(
    backend: &dyn Backend,
    model_name: &str,
    steps: &[smelt_planner::ExecutionStep],
    schema: &str,
    partition: PartitionRange,
    event_time_column: &str,
    time_range: &smelt_runtime::TimeRange,
    inc_strategy: IncrementalStrategy,
    unique_key: Vec<String>,
    show_results: bool,
) -> Result<ExecutionResult> {
    use smelt_runtime::inject_time_filter;

    let start = std::time::Instant::now();

    let table_exists = backend
        .table_exists(schema, model_name)
        .await
        .unwrap_or(false);

    // For DELETE+INSERT strategy, delete partitions upfront before inserting
    if table_exists && inc_strategy == IncrementalStrategy::DeleteInsert {
        backend
            .delete_partitions(schema, model_name, &partition)
            .await
            .map_err(|e| CliError::ExecutionError {
                model: model_name.to_string(),
                sql: "DELETE partitions".to_string(),
                source: e.into(),
            })?;
    }

    for step in steps {
        match step {
            smelt_planner::ExecutionStep::CreateTemp { name, sql } => {
                let filtered_sql = inject_time_filter(sql, event_time_column, time_range)
                    .map_err(|e| anyhow::anyhow!("Failed to inject time filter: {}", e))?;
                let create_sql = format!("CREATE TEMP TABLE {} AS {}", name, filtered_sql);
                backend
                    .execute_sql(&create_sql)
                    .await
                    .map_err(|e| CliError::ExecutionError {
                        model: model_name.to_string(),
                        sql: create_sql.clone(),
                        source: e.into(),
                    })?;
            }
            smelt_planner::ExecutionStep::AppendToTemp { name, sql } => {
                let filtered_sql = inject_time_filter(sql, event_time_column, time_range)
                    .map_err(|e| anyhow::anyhow!("Failed to inject time filter: {}", e))?;
                let insert_sql = format!("INSERT INTO {} {}", name, filtered_sql);
                backend
                    .execute_sql(&insert_sql)
                    .await
                    .map_err(|e| CliError::ExecutionError {
                        model: model_name.to_string(),
                        sql: insert_sql.clone(),
                        source: e.into(),
                    })?;
            }
            smelt_planner::ExecutionStep::FinalQuery { sql } => {
                if !table_exists {
                    backend
                        .create_table_as(schema, model_name, sql)
                        .await
                        .map_err(|e| CliError::ExecutionError {
                            model: model_name.to_string(),
                            sql: sql.clone(),
                            source: e.into(),
                        })?;
                } else {
                    let _ = &unique_key; // reserved for future audit/logging use
                    match inc_strategy {
                        IncrementalStrategy::DeleteInsert => {
                            // Partitions already deleted above
                            backend
                                .insert_into_from_query(schema, model_name, sql)
                                .await
                                .map_err(|e| CliError::ExecutionError {
                                    model: model_name.to_string(),
                                    sql: sql.clone(),
                                    source: e.into(),
                                })?;
                        }
                        IncrementalStrategy::Append => {
                            backend
                                .insert_into_from_query(schema, model_name, sql)
                                .await
                                .map_err(|e| CliError::ExecutionError {
                                    model: model_name.to_string(),
                                    sql: sql.clone(),
                                    source: e.into(),
                                })?;
                        }
                        IncrementalStrategy::InsertOverwrite => {
                            backend
                                .insert_overwrite(schema, model_name, sql, &partition)
                                .await
                                .map_err(|e| CliError::ExecutionError {
                                    model: model_name.to_string(),
                                    sql: sql.clone(),
                                    source: e.into(),
                                })?;
                        }
                    }
                }
            }
            smelt_planner::ExecutionStep::DropTemp { name } => {
                let drop_sql = format!("DROP TABLE IF EXISTS {}", name);
                let _ = backend.execute_sql(&drop_sql).await;
            }
        }
    }

    let duration = start.elapsed();
    let row_count = backend.get_row_count(schema, model_name).await.unwrap_or(0);

    let preview = if show_results {
        backend.get_preview(schema, model_name, 10).await.ok()
    } else {
        None
    };

    Ok(ExecutionResult {
        model_name: model_name.to_string(),
        duration,
        row_count,
        preview,
    })
}

/// Validate that all source tables exist in the backend.
pub async fn validate_sources(backend: &dyn Backend, sources: &SourcesConfig) -> Result<()> {
    let mut missing = Vec::new();

    for source in &sources.sources {
        for table in &source.tables {
            let exists = backend
                .table_exists(&source.name, &table.name)
                .await
                .unwrap_or(false);

            if !exists {
                missing.push(format!("{}.{}", source.name, table.name));
            }
        }
    }

    if !missing.is_empty() {
        return Err(CliError::SourceTablesNotFound { missing }.into());
    }

    Ok(())
}

#[cfg(test)]
#[cfg(feature = "duckdb")]
mod tests {
    use super::*;
    use smelt_backend_duckdb::DuckDbBackend;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_executor_creation() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.duckdb");

        let _backend = DuckDbBackend::new(&db_path, "main").await.unwrap();
        assert!(db_path.exists());
    }

    #[tokio::test]
    async fn test_execute_table() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.duckdb");

        let backend = DuckDbBackend::new(&db_path, "main").await.unwrap();

        let compiled = CompiledModel {
            name: "test_model".to_string(),
            sql: "SELECT 1 as id, 'test' as name".to_string(),
            materialization: crate::config::Materialization::Table,
        };

        let result = execute_model(&backend, &compiled, "main", false)
            .await
            .unwrap();

        assert_eq!(result.model_name, "test_model");
        assert_eq!(result.row_count, 1);
        assert!(result.preview.is_none());
    }

    #[tokio::test]
    async fn test_execute_view() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.duckdb");

        let backend = DuckDbBackend::new(&db_path, "main").await.unwrap();

        let compiled = CompiledModel {
            name: "test_view".to_string(),
            sql: "SELECT 1 as id, 'test' as name".to_string(),
            materialization: crate::config::Materialization::View,
        };

        let result = execute_model(&backend, &compiled, "main", false)
            .await
            .unwrap();

        assert_eq!(result.model_name, "test_view");
        assert_eq!(result.row_count, 1);
    }

    #[tokio::test]
    async fn test_execute_with_preview() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.duckdb");

        let backend = DuckDbBackend::new(&db_path, "main").await.unwrap();

        let compiled = CompiledModel {
            name: "test_preview".to_string(),
            sql: "SELECT 1 as id UNION SELECT 2 UNION SELECT 3".to_string(),
            materialization: crate::config::Materialization::Table,
        };

        let result = execute_model(&backend, &compiled, "main", true)
            .await
            .unwrap();

        assert_eq!(result.row_count, 3);
        assert!(result.preview.is_some());

        let batches = result.preview.unwrap();
        let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total_rows, 3);
    }
}
