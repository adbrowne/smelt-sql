use crate::compiler::CompiledModel;
use crate::config::SourceConfig;
use crate::errors::CliError;
use anyhow::Result;
use smelt_backend::{Backend, ExecutionResult, Materialization};

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
    };

    backend
        .execute_model(schema, &compiled.name, &compiled.sql, materialization, show_results)
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

/// Validate that all source tables exist in the backend.
pub async fn validate_sources(
    backend: &dyn Backend,
    sources: &SourceConfig,
) -> Result<()> {
    let mut missing = Vec::new();

    for (schema_name, schema) in &sources.sources {
        for table_name in schema.tables.keys() {
            let exists = backend
                .table_exists(schema_name, table_name)
                .await
                .unwrap_or(false);

            if !exists {
                missing.push(format!("{}.{}", schema_name, table_name));
            }
        }
    }

    if !missing.is_empty() {
        return Err(CliError::SourceTablesNotFound { missing }.into());
    }

    Ok(())
}

#[cfg(test)]
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

        let result = execute_model(&backend, &compiled, "main", false).await.unwrap();

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

        let result = execute_model(&backend, &compiled, "main", false).await.unwrap();

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

        let result = execute_model(&backend, &compiled, "main", true).await.unwrap();

        assert_eq!(result.row_count, 3);
        assert!(result.preview.is_some());

        let batches = result.preview.unwrap();
        let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total_rows, 3);
    }
}
