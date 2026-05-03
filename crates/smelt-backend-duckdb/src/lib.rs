//! DuckDB backend implementation for smelt.

use anyhow::Context;
use arrow::array::RecordBatch;
use arrow::datatypes::{DataType, SchemaRef, TimeUnit};
use async_trait::async_trait;
use duckdb::Connection;
use smelt_backend::{Backend, BackendCapabilities, BackendError, PartitionSpec, SqlDialect};
use std::path::Path;
use std::sync::{Arc, Mutex};

/// Map an Arrow `DataType` to a DuckDB DDL type string.
///
/// Covers the seed type set from `seeds.md §"Type inference"`:
/// BOOLEAN, INTEGER, BIGINT, DECIMAL(p, s), DOUBLE, DATE, TIMESTAMP (no TZ), VARCHAR.
fn arrow_type_to_duckdb_ddl(dt: &DataType) -> Result<String, String> {
    match dt {
        DataType::Boolean => Ok("BOOLEAN".to_string()),
        DataType::Int32 => Ok("INTEGER".to_string()),
        DataType::Int64 => Ok("BIGINT".to_string()),
        DataType::Decimal128(p, s) if *p <= 18 && *s <= 4 => Ok(format!("DECIMAL({}, {})", p, s)),
        DataType::Decimal128(p, s) => Err(format!(
            "DECIMAL({}, {}) exceeds supported bounds (p≤18, s≤4); use DOUBLE instead",
            p, s
        )),
        DataType::Float64 => Ok("DOUBLE".to_string()),
        DataType::Date32 => Ok("DATE".to_string()),
        DataType::Timestamp(TimeUnit::Microsecond, None) => Ok("TIMESTAMP".to_string()),
        DataType::Utf8 => Ok("VARCHAR".to_string()),
        other => Err(format!(
            "unsupported Arrow type for load_table: {:?}",
            other
        )),
    }
}

/// DuckDB backend for smelt.
///
/// Wraps a DuckDB connection and implements the Backend trait.
/// DuckDB operations are synchronous, so they're wrapped in spawn_blocking.
/// Uses Arc<Mutex<Connection>> since Connection is not Sync.
pub struct DuckDbBackend {
    connection: Arc<Mutex<Connection>>,
    #[allow(dead_code)] // Used in new() for schema creation
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
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("Failed to create directory: {:?}", parent))?;
            }

            // Open file-based connection (persistent)
            let connection = Connection::open(&database_path)
                .with_context(|| format!("Failed to open DuckDB database: {:?}", database_path))?;

            // Ensure schema exists
            connection
                .execute(
                    &format!("CREATE SCHEMA IF NOT EXISTS {}", schema_for_init),
                    [],
                )
                .with_context(|| format!("Failed to create schema: {}", schema_for_init))?;

            Ok::<_, anyhow::Error>(Arc::new(Mutex::new(connection)))
        })
        .await
        .map_err(|e| BackendError::connection_failed(e.to_string()))?
        .map_err(|e| BackendError::connection_failed(e.to_string()))?;

        Ok(Self { connection, schema })
    }

    /// Check if a table exists in the information schema.
    pub async fn table_exists_sync(
        &self,
        schema: &str,
        table_name: &str,
    ) -> Result<bool, BackendError> {
        let query = "SELECT COUNT(*) > 0 FROM information_schema.tables WHERE table_schema = ? AND table_name = ?";
        let connection = Arc::clone(&self.connection);
        let schema = schema.to_string();
        let table_name = table_name.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = connection.lock().expect("DuckDB connection mutex poisoned");
            conn.query_row(query, [&schema, &table_name], |row| row.get(0))
                .unwrap_or(false)
        })
        .await
        .map_err(|e| BackendError::Other(e.into()))
    }

    /// Probe `information_schema.tables` for the catalog object kind.
    ///
    /// Returns `Some("BASE TABLE")` for tables, `Some("VIEW")` for views, and
    /// `None` if no object with that name exists in the given schema.
    ///
    /// This is the type-aware probe needed before issuing a `DROP` statement:
    /// DuckDB's `DROP TABLE/VIEW IF EXISTS` only guards on existence — it
    /// raises a Catalog Error when the named object is of the wrong kind.
    /// Callers should issue the `DROP` matching the returned kind.
    async fn probe_object_kind(
        &self,
        schema: &str,
        name: &str,
    ) -> Result<Option<String>, BackendError> {
        let query = "SELECT table_type FROM information_schema.tables \
                     WHERE table_schema = ? AND table_name = ? LIMIT 1";
        let connection = Arc::clone(&self.connection);
        let schema = schema.to_string();
        let name = name.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = connection.lock().expect("DuckDB connection mutex poisoned");
            match conn.query_row(query, [&schema, &name], |row| row.get::<_, String>(0)) {
                Ok(kind) => Ok(Some(kind)),
                Err(duckdb::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(BackendError::execution_failed(
                    format!("{}.{}", schema, name),
                    e.to_string(),
                )),
            }
        })
        .await
        .map_err(|e| BackendError::Other(e.into()))?
    }
}

#[async_trait]
impl Backend for DuckDbBackend {
    async fn execute_sql(&self, sql: &str) -> Result<Vec<RecordBatch>, BackendError> {
        let connection = Arc::clone(&self.connection);
        let sql = sql.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = connection.lock().expect("DuckDB connection mutex poisoned");
            let mut stmt = conn
                .prepare(&sql)
                .map_err(|e| BackendError::execution_failed("query", e.to_string()))?;

            let result = stmt
                .query_arrow([])
                .map_err(|e| BackendError::execution_failed("query", e.to_string()))?;

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
            let conn = connection.lock().expect("DuckDB connection mutex poisoned");
            conn.execute(&create_sql, [])
                .map_err(|e| BackendError::execution_failed(table_name.clone(), e.to_string()))?;
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
            let conn = connection.lock().expect("DuckDB connection mutex poisoned");
            conn.execute(&create_sql, [])
                .map_err(|e| BackendError::execution_failed(view_name.clone(), e.to_string()))?;
            Ok(())
        })
        .await
        .map_err(|e| BackendError::Other(e.into()))?
    }

    async fn drop_table_if_exists(&self, schema: &str, name: &str) -> Result<(), BackendError> {
        // Probe the catalog: only issue `DROP TABLE` if the named object is
        // actually a Table. DuckDB's `DROP TABLE IF EXISTS` raises a Catalog
        // Error when the object exists but is a View — `IF EXISTS` guards on
        // existence, not type. Doing nothing for views/missing objects here
        // matches the semantics callers expect (they pair this with
        // `drop_view_if_exists`).
        let kind = self.probe_object_kind(schema, name).await?;
        if !matches!(kind.as_deref(), Some("BASE TABLE")) {
            return Ok(());
        }

        let table_name = format!("{}.{}", schema, name);
        let drop_sql = format!("DROP TABLE {}", table_name);
        let connection = Arc::clone(&self.connection);

        tokio::task::spawn_blocking(move || {
            let conn = connection.lock().expect("DuckDB connection mutex poisoned");
            conn.execute(&drop_sql, [])
                .map_err(|e| BackendError::execution_failed(table_name.clone(), e.to_string()))?;
            Ok(())
        })
        .await
        .map_err(|e| BackendError::Other(e.into()))?
    }

    async fn drop_view_if_exists(&self, schema: &str, name: &str) -> Result<(), BackendError> {
        // Mirror of `drop_table_if_exists`: only issue `DROP VIEW` when the
        // catalog object is actually a View. See that method for rationale.
        let kind = self.probe_object_kind(schema, name).await?;
        if !matches!(kind.as_deref(), Some("VIEW")) {
            return Ok(());
        }

        let view_name = format!("{}.{}", schema, name);
        let drop_sql = format!("DROP VIEW {}", view_name);
        let connection = Arc::clone(&self.connection);

        tokio::task::spawn_blocking(move || {
            let conn = connection.lock().expect("DuckDB connection mutex poisoned");
            conn.execute(&drop_sql, [])
                .map_err(|e| BackendError::execution_failed(view_name.clone(), e.to_string()))?;
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
            let conn = connection.lock().expect("DuckDB connection mutex poisoned");
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
            let conn = connection.lock().expect("DuckDB connection mutex poisoned");
            let mut stmt = conn
                .prepare(&sql)
                .map_err(|e| BackendError::execution_failed(table_name.clone(), e.to_string()))?;

            let result = stmt
                .query_arrow([])
                .map_err(|e| BackendError::execution_failed(table_name.clone(), e.to_string()))?;

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
            let conn = connection.lock().expect("DuckDB connection mutex poisoned");
            conn.execute(&sql, [])
                .map_err(|e| BackendError::execution_failed("schema", e.to_string()))?;
            Ok(())
        })
        .await
        .map_err(|e| BackendError::Other(e.into()))?
    }

    async fn load_table(
        &self,
        schema: &str,
        name: &str,
        arrow_schema: SchemaRef,
        batches: Vec<RecordBatch>,
    ) -> Result<(), BackendError> {
        let table_name = format!("{}.{}", schema, name);
        let schema_str = schema.to_string();
        let name_str = name.to_string();
        let connection = Arc::clone(&self.connection);

        // Pre-validate nullability against the authoritative `arrow_schema`.
        // The batch may have been constructed with looser (nullable: true) field
        // declarations; we honour the nullability declared in `arrow_schema`.
        for batch in &batches {
            for (col_idx, field) in arrow_schema.fields().iter().enumerate() {
                if !field.is_nullable() {
                    let array = batch.column(col_idx);
                    if array.null_count() > 0 {
                        // Find the first null row for the error message.
                        let row = (0..array.len()).find(|&i| array.is_null(i)).unwrap_or(0);
                        return Err(BackendError::null_in_non_nullable_column(
                            schema,
                            name,
                            field.name().as_str(),
                            row,
                        ));
                    }
                }
            }
        }

        // Build the CREATE TABLE DDL from the Arrow schema.
        let ddl = {
            let mut cols = Vec::with_capacity(arrow_schema.fields().len());
            for field in arrow_schema.fields() {
                let duckdb_type = arrow_type_to_duckdb_ddl(field.data_type())
                    .map_err(|msg| BackendError::execution_failed(table_name.clone(), msg))?;
                let nullability = if field.is_nullable() {
                    "NULL"
                } else {
                    "NOT NULL"
                };
                cols.push(format!("{} {} {}", field.name(), duckdb_type, nullability));
            }
            format!(
                "CREATE TABLE {}.{} ({})",
                schema_str,
                name_str,
                cols.join(", ")
            )
        };

        tokio::task::spawn_blocking(move || {
            let conn = connection.lock().expect("DuckDB connection mutex poisoned");

            // Drop any existing table or view with the same name.
            let probe_sql = "SELECT table_type FROM information_schema.tables \
                             WHERE table_schema = ? AND table_name = ? LIMIT 1";
            let kind: Option<String> =
                match conn.query_row(probe_sql, [&schema_str, &name_str], |row| row.get(0)) {
                    Ok(k) => Some(k),
                    Err(duckdb::Error::QueryReturnedNoRows) => None,
                    Err(e) => {
                        return Err(BackendError::execution_failed(
                            format!("{}.{}", schema_str, name_str),
                            e.to_string(),
                        ))
                    }
                };
            match kind.as_deref() {
                Some("BASE TABLE") => {
                    conn.execute(&format!("DROP TABLE {}.{}", schema_str, name_str), [])
                        .map_err(|e| {
                            BackendError::execution_failed(table_name.clone(), e.to_string())
                        })?;
                }
                Some("VIEW") => {
                    conn.execute(&format!("DROP VIEW {}.{}", schema_str, name_str), [])
                        .map_err(|e| {
                            BackendError::execution_failed(table_name.clone(), e.to_string())
                        })?;
                }
                _ => {}
            }

            // Create the table.
            conn.execute(&ddl, [])
                .map_err(|e| BackendError::execution_failed(table_name.clone(), e.to_string()))?;

            // Append each batch via the Arrow appender.
            let mut appender = conn
                .appender_to_db(&name_str, &schema_str)
                .map_err(|e| BackendError::execution_failed(table_name.clone(), e.to_string()))?;

            for batch in batches {
                appender.append_record_batch(batch).map_err(|e| {
                    BackendError::execution_failed(table_name.clone(), e.to_string())
                })?;
            }

            appender
                .flush()
                .map_err(|e| BackendError::execution_failed(table_name.clone(), e.to_string()))?;

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

    async fn delete_partitions(
        &self,
        schema: &str,
        name: &str,
        partition: &PartitionSpec,
    ) -> Result<(), BackendError> {
        let table_name = format!("{}.{}", schema, name);

        // Build WHERE clause: column IN ('value1', 'value2', ...)
        let values_list = partition
            .values
            .iter()
            .map(|v| format!("'{}'", v.replace("'", "''"))) // SQL escape
            .collect::<Vec<_>>()
            .join(", ");

        let delete_sql = format!(
            "DELETE FROM {} WHERE {} IN ({})",
            table_name, partition.column, values_list
        );

        let connection = Arc::clone(&self.connection);

        tokio::task::spawn_blocking(move || {
            let conn = connection.lock().expect("DuckDB connection mutex poisoned");
            conn.execute(&delete_sql, [])
                .map_err(|e| BackendError::execution_failed(table_name.clone(), e.to_string()))?;
            Ok(())
        })
        .await
        .map_err(|e| BackendError::Other(e.into()))?
    }

    async fn insert_into_from_query(
        &self,
        schema: &str,
        name: &str,
        sql: &str,
    ) -> Result<(), BackendError> {
        let table_name = format!("{}.{}", schema, name);
        let insert_sql = format!("INSERT INTO {} {}", table_name, sql);
        let connection = Arc::clone(&self.connection);

        tokio::task::spawn_blocking(move || {
            let conn = connection.lock().expect("DuckDB connection mutex poisoned");
            conn.execute(&insert_sql, [])
                .map_err(|e| BackendError::execution_failed(table_name.clone(), e.to_string()))?;
            Ok(())
        })
        .await
        .map_err(|e| BackendError::Other(e.into()))?
    }

    async fn merge_into(
        &self,
        schema: &str,
        table: &str,
        source_sql: &str,
        unique_key: &[String],
    ) -> Result<(), BackendError> {
        let table_name = format!("{}.{}", schema, table);

        let on_clause = unique_key
            .iter()
            .map(|k| format!("target.{} = source.{}", k, k))
            .collect::<Vec<_>>()
            .join(" AND ");

        let merge_sql = format!(
            "MERGE INTO {} AS target USING ({}) AS source ON {} \
             WHEN MATCHED THEN UPDATE SET * \
             WHEN NOT MATCHED THEN INSERT *",
            table_name, source_sql, on_clause
        );

        let connection = Arc::clone(&self.connection);

        tokio::task::spawn_blocking(move || {
            let conn = connection.lock().expect("DuckDB connection mutex poisoned");
            conn.execute(&merge_sql, [])
                .map_err(|e| BackendError::execution_failed(table_name.clone(), e.to_string()))?;
            Ok(())
        })
        .await
        .map_err(|e| BackendError::Other(e.into()))?
    }

    async fn insert_overwrite(
        &self,
        schema: &str,
        table: &str,
        sql: &str,
        partition: &PartitionSpec,
    ) -> Result<(), BackendError> {
        let table_name = format!("{}.{}", schema, table);

        // DuckDB has no native INSERT OVERWRITE. Emulate by deleting partition
        // values that appear in the source query, then inserting.
        let delete_sql = format!(
            "DELETE FROM {} WHERE {} IN (SELECT DISTINCT {} FROM ({}))",
            table_name, partition.column, partition.column, sql
        );

        let insert_sql = format!("INSERT INTO {} {}", table_name, sql);

        let connection = Arc::clone(&self.connection);

        tokio::task::spawn_blocking(move || {
            let conn = connection.lock().expect("DuckDB connection mutex poisoned");
            conn.execute(&delete_sql, [])
                .map_err(|e| BackendError::execution_failed(table_name.clone(), e.to_string()))?;
            conn.execute(&insert_sql, [])
                .map_err(|e| BackendError::execution_failed(table_name.clone(), e.to_string()))?;
            Ok(())
        })
        .await
        .map_err(|e| BackendError::Other(e.into()))?
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
        assert!(!caps.supports_insert_overwrite);
    }

    #[tokio::test]
    async fn test_merge_into_upsert() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.duckdb");

        let backend = DuckDbBackend::new(&db_path, "main").await.unwrap();

        // Create initial table
        backend
            .execute_sql("CREATE TABLE main.users AS SELECT 1 as id, 'Alice' as name, 100 as score")
            .await
            .unwrap();

        // MERGE: update existing row (id=1) and insert new row (id=2)
        backend
            .merge_into(
                "main",
                "users",
                "SELECT * FROM (VALUES (1, 'Alice', 200), (2, 'Bob', 150)) AS t(id, name, score)",
                &["id".to_string()],
            )
            .await
            .unwrap();

        let count = backend.get_row_count("main", "users").await.unwrap();
        assert_eq!(count, 2, "Expected 2 rows after merge");

        // Verify Alice's score was updated
        let result = backend
            .execute_sql("SELECT score FROM main.users WHERE id = 1")
            .await
            .unwrap();
        let score: i32 = result[0]
            .column(0)
            .as_any()
            .downcast_ref::<arrow::array::Int32Array>()
            .unwrap()
            .value(0);
        assert_eq!(score, 200, "Expected Alice's score to be updated to 200");
    }

    #[tokio::test]
    async fn test_merge_into_insert_only() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.duckdb");

        let backend = DuckDbBackend::new(&db_path, "main").await.unwrap();

        // Create initial table with one row
        backend
            .execute_sql("CREATE TABLE main.items AS SELECT 1 as id, 'A' as name")
            .await
            .unwrap();

        // MERGE with only new rows
        backend
            .merge_into(
                "main",
                "items",
                "SELECT * FROM (VALUES (2, 'B'), (3, 'C')) AS t(id, name)",
                &["id".to_string()],
            )
            .await
            .unwrap();

        let count = backend.get_row_count("main", "items").await.unwrap();
        assert_eq!(count, 3, "Expected 3 rows after merge insert");
    }

    #[tokio::test]
    async fn test_insert_overwrite() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.duckdb");

        let backend = DuckDbBackend::new(&db_path, "main").await.unwrap();

        // Create initial table with data across multiple dates
        backend
            .execute_sql(
                "CREATE TABLE main.daily AS SELECT * FROM (VALUES \
                 ('2024-01-01', 10), ('2024-01-01', 20), \
                 ('2024-01-02', 30), \
                 ('2024-01-03', 40) \
                 ) AS t(dt, val)",
            )
            .await
            .unwrap();

        let initial_count = backend.get_row_count("main", "daily").await.unwrap();
        assert_eq!(initial_count, 4);

        // INSERT OVERWRITE for dt='2024-01-01' — replaces 2 rows with 1
        let partition = smelt_backend::PartitionSpec {
            column: "dt".to_string(),
            values: vec!["2024-01-01".to_string()],
        };

        backend
            .insert_overwrite(
                "main",
                "daily",
                "SELECT '2024-01-01' as dt, 999 as val",
                &partition,
            )
            .await
            .unwrap();

        let count = backend.get_row_count("main", "daily").await.unwrap();
        assert_eq!(
            count, 3,
            "Expected 3 rows: 1 replaced + 1 for Jan 2 + 1 for Jan 3"
        );

        // Verify the overwritten value
        let result = backend
            .execute_sql("SELECT val FROM main.daily WHERE dt = '2024-01-01'")
            .await
            .unwrap();
        let val: i32 = result[0]
            .column(0)
            .as_any()
            .downcast_ref::<arrow::array::Int32Array>()
            .unwrap()
            .value(0);
        assert_eq!(val, 999);
    }

    #[tokio::test]
    async fn test_resolve_strategy_with_unique_key() {
        use smelt_backend::IncrementalConfig;
        use smelt_backend::{Granularity, IncrementalSafetyOverrides};

        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.duckdb");
        let backend = DuckDbBackend::new(&db_path, "main").await.unwrap();

        let config = IncrementalConfig {
            enabled: true,
            event_time_column: "ts".to_string(),
            partition_column: "dt".to_string(),
            granularity: Granularity::Day,
            unique_key: vec!["id".to_string()],
            safety_overrides: IncrementalSafetyOverrides::default(),
        };

        let strategy = backend.resolve_strategy(&config);
        assert_eq!(strategy, smelt_backend::IncrementalStrategy::Merge);
    }

    #[tokio::test]
    async fn test_resolve_strategy_without_unique_key() {
        use smelt_backend::IncrementalConfig;
        use smelt_backend::{Granularity, IncrementalSafetyOverrides};

        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.duckdb");
        let backend = DuckDbBackend::new(&db_path, "main").await.unwrap();

        let config = IncrementalConfig {
            enabled: true,
            event_time_column: "ts".to_string(),
            partition_column: "dt".to_string(),
            granularity: Granularity::Day,
            unique_key: vec![],
            safety_overrides: IncrementalSafetyOverrides::default(),
        };

        let strategy = backend.resolve_strategy(&config);
        assert_eq!(strategy, smelt_backend::IncrementalStrategy::DeleteInsert);
    }

    /// Bug #1 (re-run a Table materialization):
    /// Re-creating a model that exists as a Table must succeed when
    /// `execute_model` is called a second time. Phase 7's prior fix issued
    /// `DROP VIEW IF EXISTS` before `DROP TABLE IF EXISTS`, but DuckDB rejects
    /// `DROP VIEW IF EXISTS` against an existing Table — `IF EXISTS` guards on
    /// existence, not on type.
    #[tokio::test]
    async fn test_recreate_existing_table() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.duckdb");
        let backend = DuckDbBackend::new(&db_path, "main").await.unwrap();

        // First execution: creates `t` as a Table with column `(a INT)`.
        backend
            .execute_model("main", "t", "SELECT 1 AS a", Materialization::Table, false)
            .await
            .expect("first table create");

        // Second execution: re-creates `t` with new contents/schema. The
        // catalog already holds a Table named `t`. This should succeed.
        backend
            .execute_model(
                "main",
                "t",
                "SELECT 2 AS a, 3 AS b",
                Materialization::Table,
                false,
            )
            .await
            .expect("second table create (idempotency)");

        // Verify the new contents are present.
        let row_count = backend.get_row_count("main", "t").await.unwrap();
        assert_eq!(row_count, 1);
        let batches = backend
            .execute_sql("SELECT a, b FROM main.t")
            .await
            .unwrap();
        let a: i32 = batches[0]
            .column(0)
            .as_any()
            .downcast_ref::<arrow::array::Int32Array>()
            .unwrap()
            .value(0);
        let b: i32 = batches[0]
            .column(1)
            .as_any()
            .downcast_ref::<arrow::array::Int32Array>()
            .unwrap()
            .value(0);
        assert_eq!(a, 2);
        assert_eq!(b, 3);
    }

    /// Bug #1 (materialization change view -> table):
    /// A model previously materialized as a View, then re-run with
    /// `materialization: table`, must succeed. The Phase 7 logic correctly
    /// drops the View first here, then creates a Table.
    #[tokio::test]
    async fn test_recreate_after_view_to_table_change() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.duckdb");
        let backend = DuckDbBackend::new(&db_path, "main").await.unwrap();

        backend
            .execute_model("main", "t", "SELECT 1 AS a", Materialization::View, false)
            .await
            .expect("create as view");

        backend
            .execute_model("main", "t", "SELECT 2 AS a", Materialization::Table, false)
            .await
            .expect("recreate as table after view");

        let row: String = duckdb::Connection::open(&db_path)
            .unwrap()
            .query_row(
                "SELECT table_type FROM information_schema.tables \
                 WHERE table_schema = 'main' AND table_name = 't'",
                [],
                |row| row.get(0),
            )
            .expect("read table_type");
        assert_eq!(row, "BASE TABLE");
    }

    /// Bug #1 (materialization change table -> view):
    /// Symmetric to the previous test — a model previously materialized as a
    /// Table, then re-run with `materialization: view`, must succeed.
    #[tokio::test]
    async fn test_recreate_after_table_to_view_change() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.duckdb");
        let backend = DuckDbBackend::new(&db_path, "main").await.unwrap();

        backend
            .execute_model("main", "t", "SELECT 1 AS a", Materialization::Table, false)
            .await
            .expect("create as table");

        backend
            .execute_model("main", "t", "SELECT 2 AS a", Materialization::View, false)
            .await
            .expect("recreate as view after table");

        let row: String = duckdb::Connection::open(&db_path)
            .unwrap()
            .query_row(
                "SELECT table_type FROM information_schema.tables \
                 WHERE table_schema = 'main' AND table_name = 't'",
                [],
                |row| row.get(0),
            )
            .expect("read table_type");
        assert_eq!(row, "VIEW");
    }
}
