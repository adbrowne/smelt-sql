//! DuckDB backend implementation for smelt.

use anyhow::Context;
use arrow::array::RecordBatch;
use async_trait::async_trait;
use duckdb::Connection;
use smelt_backend::{Backend, BackendCapabilities, BackendError, SqlDialect};
use std::path::Path;
use std::sync::{Arc, Mutex};

/// DuckDB backend for smelt.
///
/// Wraps a DuckDB connection and implements the Backend trait.
/// DuckDB operations are synchronous, so they're wrapped in spawn_blocking.
/// Uses Arc<Mutex<Connection>> since Connection is not Sync.
pub struct DuckDbBackend {
    connection: Arc<Mutex<Connection>>,
    schema: String,
}

impl DuckDbBackend {
    /// Create a new DuckDB backend.
    ///
    /// Opens or creates a database file at the given path and ensures the schema exists.
    pub async fn new(database_path: &Path, schema: &str) -> Result<Self, BackendError> {
        let database_path = database_path.to_owned();
        let schema = schema.to_string();
        let schema_for_init = schema.clone();

        // Run blocking DuckDB operations in spawn_blocking
        let connection = tokio::task::spawn_blocking(move || {
            // Create parent directory if needed
            if let Some(parent) = database_path.parent() {
                std::fs::create_dir_all(parent).with_context(|| {
                    format!("Failed to create directory: {:?}", parent)
                })?;
            }

            // Open file-based connection (persistent)
            let connection = Connection::open(&database_path).with_context(|| {
                format!("Failed to open DuckDB database: {:?}", database_path)
            })?;

            // Ensure schema exists
            connection
                .execute(&format!("CREATE SCHEMA IF NOT EXISTS {}", schema_for_init), [])
                .with_context(|| format!("Failed to create schema: {}", schema_for_init))?;

            Ok::<_, anyhow::Error>(Arc::new(Mutex::new(connection)))
        })
        .await
        .map_err(|e| BackendError::connection_failed(e.to_string()))?
        .map_err(|e| BackendError::connection_failed(e.to_string()))?;

        Ok(Self {
            connection,
            schema,
        })
    }

    /// Check if a table exists in the information schema.
    pub async fn table_exists_sync(&self, schema: &str, table_name: &str) -> Result<bool, BackendError> {
        let query = "SELECT COUNT(*) > 0 FROM information_schema.tables WHERE table_schema = ? AND table_name = ?";
        let connection = Arc::clone(&self.connection);
        let schema = schema.to_string();
        let table_name = table_name.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = connection.lock().unwrap();
            conn.query_row(query, [&schema, &table_name], |row| row.get(0))
                .unwrap_or(false)
        })
        .await
        .map_err(|e| BackendError::Other(e.into()))
    }
}

#[async_trait]
impl Backend for DuckDbBackend {
    async fn execute_sql(&self, sql: &str) -> Result<Vec<RecordBatch>, BackendError> {
        let connection = Arc::clone(&self.connection);
        let sql = sql.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = connection.lock().unwrap();
            let mut stmt = conn.prepare(&sql).map_err(|e| {
                BackendError::execution_failed("query", e.to_string())
            })?;

            let result = stmt.query_arrow([]).map_err(|e| {
                BackendError::execution_failed("query", e.to_string())
            })?;

            Ok(result.collect())
        })
        .await
        .map_err(|e| BackendError::Other(e.into()))?
    }

    async fn create_table_as(
        &self,
        schema: &str,
        name: &str,
        sql: &str,
    ) -> Result<(), BackendError> {
        let table_name = format!("{}.{}", schema, name);
        let create_sql = format!("CREATE TABLE {} AS {}", table_name, sql);
        let connection = Arc::clone(&self.connection);

        tokio::task::spawn_blocking(move || {
            let conn = connection.lock().unwrap();
            conn.execute(&create_sql, []).map_err(|e| {
                BackendError::execution_failed(table_name.clone(), e.to_string())
            })?;
            Ok(())
        })
        .await
        .map_err(|e| BackendError::Other(e.into()))?
    }

    async fn create_view_as(
        &self,
        schema: &str,
        name: &str,
        sql: &str,
    ) -> Result<(), BackendError> {
        let view_name = format!("{}.{}", schema, name);
        let create_sql = format!("CREATE VIEW {} AS {}", view_name, sql);
        let connection = Arc::clone(&self.connection);

        tokio::task::spawn_blocking(move || {
            let conn = connection.lock().unwrap();
            conn.execute(&create_sql, []).map_err(|e| {
                BackendError::execution_failed(view_name.clone(), e.to_string())
            })?;
            Ok(())
        })
        .await
        .map_err(|e| BackendError::Other(e.into()))?
    }

    async fn drop_table_if_exists(&self, schema: &str, name: &str) -> Result<(), BackendError> {
        let table_name = format!("{}.{}", schema, name);
        let drop_sql = format!("DROP TABLE IF EXISTS {}", table_name);
        let connection = Arc::clone(&self.connection);

        tokio::task::spawn_blocking(move || {
            let conn = connection.lock().unwrap();
            conn.execute(&drop_sql, []).map_err(|e| {
                BackendError::execution_failed(table_name.clone(), e.to_string())
            })?;
            Ok(())
        })
        .await
        .map_err(|e| BackendError::Other(e.into()))?
    }

    async fn drop_view_if_exists(&self, schema: &str, name: &str) -> Result<(), BackendError> {
        let view_name = format!("{}.{}", schema, name);
        let drop_sql = format!("DROP VIEW IF EXISTS {}", view_name);
        let connection = Arc::clone(&self.connection);

        tokio::task::spawn_blocking(move || {
            let conn = connection.lock().unwrap();
            conn.execute(&drop_sql, []).map_err(|e| {
                BackendError::execution_failed(view_name.clone(), e.to_string())
            })?;
            Ok(())
        })
        .await
        .map_err(|e| BackendError::Other(e.into()))?
    }

    async fn get_row_count(&self, schema: &str, name: &str) -> Result<usize, BackendError> {
        let table_name = format!("{}.{}", schema, name);
        let sql = format!("SELECT COUNT(*) FROM {}", table_name);
        let connection = Arc::clone(&self.connection);

        tokio::task::spawn_blocking(move || {
            let conn = connection.lock().unwrap();
            conn.query_row(&sql, [], |row| row.get(0))
                .map_err(|e| BackendError::execution_failed(table_name.clone(), e.to_string()))
        })
        .await
        .map_err(|e| BackendError::Other(e.into()))?
    }

    async fn get_preview(
        &self,
        schema: &str,
        name: &str,
        limit: usize,
    ) -> Result<Vec<RecordBatch>, BackendError> {
        let table_name = format!("{}.{}", schema, name);
        let sql = format!("SELECT * FROM {} LIMIT {}", table_name, limit);
        let connection = Arc::clone(&self.connection);

        tokio::task::spawn_blocking(move || {
            let conn = connection.lock().unwrap();
            let mut stmt = conn.prepare(&sql).map_err(|e| {
                BackendError::execution_failed(table_name.clone(), e.to_string())
            })?;

            let result = stmt.query_arrow([]).map_err(|e| {
                BackendError::execution_failed(table_name.clone(), e.to_string())
            })?;

            Ok(result.collect())
        })
        .await
        .map_err(|e| BackendError::Other(e.into()))?
    }

    async fn table_exists(&self, schema: &str, name: &str) -> Result<bool, BackendError> {
        self.table_exists_sync(schema, name).await
    }

    async fn ensure_schema(&self, schema: &str) -> Result<(), BackendError> {
        let sql = format!("CREATE SCHEMA IF NOT EXISTS {}", schema);
        let connection = Arc::clone(&self.connection);

        tokio::task::spawn_blocking(move || {
            let conn = connection.lock().unwrap();
            conn.execute(&sql, [])
                .map_err(|e| BackendError::execution_failed("schema", e.to_string()))?;
            Ok(())
        })
        .await
        .map_err(|e| BackendError::Other(e.into()))?
    }

    fn dialect(&self) -> SqlDialect {
        SqlDialect::DuckDB
    }

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities::duckdb()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smelt_backend::Materialization;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_backend_creation() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.duckdb");

        let _backend = DuckDbBackend::new(&db_path, "main").await.unwrap();
        assert!(db_path.exists());
    }

    #[tokio::test]
    async fn test_execute_model_table() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.duckdb");

        let backend = DuckDbBackend::new(&db_path, "main").await.unwrap();

        let sql = "SELECT 1 as id, 'test' as name";
        let result = backend
            .execute_model("main", "test_model", sql, Materialization::Table, false)
            .await
            .unwrap();

        assert_eq!(result.model_name, "test_model");
        assert_eq!(result.row_count, 1);
        assert!(result.preview.is_none());
    }

    #[tokio::test]
    async fn test_execute_model_view() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.duckdb");

        let backend = DuckDbBackend::new(&db_path, "main").await.unwrap();

        let sql = "SELECT 1 as id, 'test' as name";
        let result = backend
            .execute_model("main", "test_view", sql, Materialization::View, false)
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

        let sql = "SELECT 1 as id UNION SELECT 2 UNION SELECT 3";
        let result = backend
            .execute_model("main", "test_preview", sql, Materialization::Table, true)
            .await
            .unwrap();

        assert_eq!(result.row_count, 3);
        assert!(result.preview.is_some());

        let batches = result.preview.unwrap();
        let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total_rows, 3);
    }

    #[tokio::test]
    async fn test_capabilities() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.duckdb");

        let backend = DuckDbBackend::new(&db_path, "main").await.unwrap();

        assert_eq!(backend.dialect(), SqlDialect::DuckDB);

        let caps = backend.capabilities();
        assert!(caps.supports_qualify);
        assert!(caps.supports_merge);
        assert!(caps.supports_create_or_replace_table);
    }
}
