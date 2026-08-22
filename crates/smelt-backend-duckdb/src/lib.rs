//! DuckDB backend implementation for smelt.

use anyhow::Context;
use arrow::array::RecordBatch;
use arrow::datatypes::{DataType, SchemaRef, TimeUnit};
use async_trait::async_trait;
use duckdb::Connection;
use smelt_backend::{
    Backend, BackendCapabilities, BackendError, PartitionRange, SqlDialect, StatementGroup,
};
use std::collections::BTreeMap;
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

/// Conservative default DuckDB `memory_limit` (in bytes) for a host with
/// `total_ram_bytes` of physical RAM.
///
/// `max(min(50% of RAM, RAM − 20 GiB), 40% of RAM)`. The smaller of the
/// 50%/`RAM−20GiB` terms keeps absolute headroom generous on large hosts and
/// proportional on small ones; the 40% floor stops the `RAM − 20 GiB` term from
/// collapsing to zero on a ≤20 GiB laptop. Deliberately conservative: DuckDB's
/// `memory_limit` bounds its buffer pool, but process RSS runs several GiB above
/// it (untracked operator/scan/Arrow memory), so the limit is set well below the
/// host to keep *RSS* within a safe envelope. See `docs/specs/smelt_yml.md`
/// §Semantics 8 for rationale.
fn default_memory_limit_bytes(total_ram_bytes: u64) -> u64 {
    const GIB: u64 = 1024 * 1024 * 1024;
    let pct50 = total_ram_bytes / 2;
    let minus_20gib = total_ram_bytes.saturating_sub(20 * GIB);
    let floor = total_ram_bytes * 4 / 10;
    pct50.min(minus_20gib).max(floor)
}

/// Best-effort total physical RAM in bytes. Linux reads `/proc/meminfo`; macOS
/// shells out to `sysctl hw.memsize`; every other platform (and any failure)
/// returns `None`, in which case smelt applies no `memory_limit` default and
/// DuckDB's own ~80%-of-RAM default stands. Never panics, never blocks.
fn detect_total_ram_bytes() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;
        for line in meminfo.lines() {
            // "MemTotal:       65536000 kB"
            if let Some(rest) = line.strip_prefix("MemTotal:") {
                let kb: u64 = rest.split_whitespace().next()?.parse().ok()?;
                return Some(kb * 1024);
            }
        }
        None
    }
    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new("sysctl")
            .args(["-n", "hw.memsize"])
            .output()
            .ok()?;
        String::from_utf8(output.stdout)
            .ok()?
            .trim()
            .parse::<u64>()
            .ok()
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        None
    }
}

/// Resolve the DuckDB connection-time settings to apply, layering smelt's
/// conservative resource defaults *under* the user's `settings:`.
///
/// A key the user set is preserved verbatim and never overridden. When absent:
/// - `memory_limit` is defaulted from `total_ram_bytes` (skipped entirely if
///   `None`, so DuckDB's native default applies);
/// - `temp_directory` is defaulted to `<database-parent>/.smelt-duckdb-tmp` so a
///   query exceeding `memory_limit` spills to disk instead of failing.
///
/// `threads` is intentionally left alone. Pure function — no I/O — so the policy
/// is unit-testable without opening a connection.
fn resolve_duckdb_settings(
    user: Option<&BTreeMap<String, String>>,
    total_ram_bytes: Option<u64>,
    database_path: &Path,
) -> BTreeMap<String, String> {
    let mut settings = user.cloned().unwrap_or_default();

    if !settings.contains_key("memory_limit") {
        if let Some(ram) = total_ram_bytes {
            let mib = default_memory_limit_bytes(ram) / (1024 * 1024);
            settings.insert("memory_limit".to_string(), format!("{}MiB", mib));
        }
    }

    if !settings.contains_key("temp_directory") {
        let parent = database_path.parent().unwrap_or_else(|| Path::new("."));
        let tmp = parent.join(".smelt-duckdb-tmp");
        settings.insert(
            "temp_directory".to_string(),
            tmp.to_string_lossy().into_owned(),
        );
    }

    settings
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
    /// Equivalent to `new_with_settings(database_path, schema, None)`.
    pub async fn new(database_path: &Path, schema: &str) -> Result<Self, BackendError> {
        Self::new_with_settings(database_path, schema, None).await
    }

    /// Create a new DuckDB backend with optional connection-time settings.
    ///
    /// Opens or creates a database file at the given path, applies each entry in
    /// `settings` as `SET key = 'value';` immediately after the connection is opened,
    /// then ensures the schema exists. Settings are applied in sorted key order.
    ///
    /// DuckDB rejects unrecognised keys natively — errors are propagated as
    /// `BackendError::connection_failed` (fail-loud discipline).
    pub async fn new_with_settings(
        database_path: &Path,
        schema: &str,
        settings: Option<&BTreeMap<String, String>>,
    ) -> Result<Self, BackendError> {
        let database_path = database_path.to_owned();
        let schema = schema.to_string();
        let schema_for_init = schema.clone();
        // Layer smelt's conservative resource defaults (memory_limit, temp_directory)
        // under the user's settings, so no single model can consume the whole host.
        // Computed here (before the move) so the pure policy stays I/O-free; the owned
        // map then crosses the spawn_blocking boundary.
        let settings_owned =
            resolve_duckdb_settings(settings, detect_total_ram_bytes(), &database_path);

        // Run blocking DuckDB operations in spawn_blocking
        let connection = tokio::task::spawn_blocking(move || {
            // Create parent directory if needed
            if let Some(parent) = database_path.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("Failed to create directory: {:?}", parent))?;
            }

            // Ensure any temp_directory (smelt-defaulted or user-set) exists before
            // we hand it to DuckDB, so spilling works on a fresh project.
            if let Some(temp_dir) = settings_owned.get("temp_directory") {
                std::fs::create_dir_all(temp_dir).with_context(|| {
                    format!("Failed to create DuckDB temp_directory: {}", temp_dir)
                })?;
            }

            // Open file-based connection (persistent)
            let connection = Connection::open(&database_path)
                .with_context(|| format!("Failed to open DuckDB database: {:?}", database_path))?;

            // Apply connection-time settings. Keys are iterated in BTreeMap order
            // (sorted) for deterministic application. DuckDB rejects unknown keys.
            for (key, value) in &settings_owned {
                connection
                    .execute(&format!("SET {} = '{}'", key, value), [])
                    .with_context(|| {
                        format!("Failed to apply DuckDB setting '{}' = '{}'", key, value)
                    })?;
            }

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
            // invariant: mutex is poisoned only when a spawn_blocking task panics; that
            // terminates the task and propagates JoinError — normal operation never poisons it.
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
            // invariant: see table_exists_sync for rationale; same mutex.
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
        partition: &PartitionRange,
    ) -> Result<(), BackendError> {
        let table_name = format!("{}.{}", schema, name);

        // Range-based DELETE: `column >= start AND column < end`.
        // More efficient than an IN-list for large windows and correct for
        // any window size.
        let delete_sql = format!(
            "DELETE FROM {} WHERE {} >= '{}' AND {} < '{}'",
            table_name,
            partition.column,
            partition.start.replace('\'', "''"),
            partition.column,
            partition.end.replace('\'', "''"),
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

    /// Real transactional override: executes the emitted [`StatementGroup`]
    /// (`smelt_logical::maintenance::emit`) inside one `duckdb::Transaction`
    /// when `group.transactional` — e.g. a paired region `DELETE`+`INSERT`
    /// (`docs/specs/incremental_models.md` §"Statement emission (single
    /// owner)"). `Transaction` rolls back on `Drop` unless explicitly
    /// committed (`duckdb::transaction::DropBehavior::Rollback` is the
    /// default), so a later statement's failure — the `?` returns before
    /// `commit()` is reached — rolls back every earlier statement in the
    /// group for free. This crate no longer builds any maintenance-statement
    /// text of its own; every string in `statements` came from the emitter.
    async fn execute_statement_group(&self, group: &StatementGroup) -> Result<(), BackendError> {
        let label = group
            .statements
            .first()
            .map(|s| s.sql.clone())
            .unwrap_or_default();
        let statements: Vec<String> = group.statements.iter().map(|s| s.sql.clone()).collect();
        let transactional = group.transactional && statements.len() > 1;
        let connection = Arc::clone(&self.connection);

        tokio::task::spawn_blocking(move || {
            // invariant: see table_exists_sync for rationale; same mutex.
            let mut conn = connection.lock().expect("DuckDB connection mutex poisoned");
            if transactional {
                let tx = conn
                    .transaction()
                    .map_err(|e| BackendError::execution_failed(label.clone(), e.to_string()))?;
                for sql in &statements {
                    tx.execute(sql, []).map_err(|e| {
                        BackendError::execution_failed(label.clone(), e.to_string())
                    })?;
                }
                tx.commit()
                    .map_err(|e| BackendError::execution_failed(label.clone(), e.to_string()))?;
            } else {
                for sql in &statements {
                    conn.execute(sql, []).map_err(|e| {
                        BackendError::execution_failed(label.clone(), e.to_string())
                    })?;
                }
            }
            Ok(())
        })
        .await
        .map_err(|e| BackendError::Other(e.into()))?
    }

    // DuckDB executes the emitted column-scoped `MERGE`'s `UPDATE SET *`
    // form; the full-row source-projection contract that shape relies on is
    // documented on `smelt_logical::maintenance::emit::
    // emit_column_scoped_merge`, the single author of that statement text.
    // `merge_into` itself is not overridden here — the `Backend` trait's
    // default implementation (build the `StatementGroup` via that emitter,
    // then `execute_statement_group`) is exactly this backend's shape: a
    // single non-transactional statement over the same connection every
    // other statement group runs through. The capability itself now lives
    // on `capabilities().supports_column_scoped_merge`
    // (`BackendCapabilities::duckdb()`, `true`), not a trait-method override.

    async fn insert_overwrite(
        &self,
        schema: &str,
        table: &str,
        sql: &str,
        partition: &PartitionRange,
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

    /// Real transactional override (`docs/specs/incremental_models.md`
    /// §Constraints "Never fold a delta already reflected in the state"):
    /// `insert_sql` (the ledger's `PRIMARY KEY`-guarded record of this
    /// delta identity) and `action_sql` (the fold itself) run inside one
    /// `duckdb::Transaction` — either both commit or neither does, so a
    /// crash between the two can never leave the ledger claiming a fold
    /// that never happened, or vice versa. `ensure_sql` (idempotent
    /// `CREATE TABLE IF NOT EXISTS`) runs first, outside that transaction —
    /// safe standalone, and keeps DuckDB's DDL-vs-constraint-check
    /// interaction out of the transaction that actually needs atomicity.
    /// A repeat delta violates the ledger table's own `PRIMARY KEY`;
    /// `Transaction`'s default `DropBehavior::Rollback` undoes the failed
    /// insert attempt for free, so `action_sql` never runs a second time —
    /// no check-then-act race across the write.
    async fn fold_ledger_delta(
        &self,
        ensure_sql: &str,
        insert_sql: &str,
        _exists_sql: &str,
        action_sql: &str,
    ) -> Result<(), BackendError> {
        let ensure_sql = ensure_sql.to_string();
        let insert_sql = insert_sql.to_string();
        let action_sql = action_sql.to_string();
        let connection = Arc::clone(&self.connection);

        tokio::task::spawn_blocking(move || {
            let mut conn = connection.lock().expect("DuckDB connection mutex poisoned");

            conn.execute(&ensure_sql, [])
                .map_err(|e| BackendError::execution_failed("ledger", e.to_string()))?;

            let tx = conn
                .transaction()
                .map_err(|e| BackendError::execution_failed("ledger", e.to_string()))?;

            if let Err(e) = tx.execute(&insert_sql, []) {
                let message = e.to_string();
                if is_constraint_violation(&message) {
                    // `tx` rolls back on drop (default `DropBehavior::Rollback`):
                    // the failed insert never lands, and `action_sql` below
                    // never runs.
                    return Err(BackendError::already_reflected(message));
                }
                return Err(BackendError::execution_failed("ledger", message));
            }

            tx.execute(&action_sql, [])
                .map_err(|e| BackendError::execution_failed("ledger", e.to_string()))?;

            tx.commit()
                .map_err(|e| BackendError::execution_failed("ledger", e.to_string()))?;
            Ok(())
        })
        .await
        .map_err(|e| BackendError::Other(e.into()))?
    }

    /// Real transactional override (`docs/specs/incremental_models.md`
    /// §"The graph layer" — "Observed deltas on model edges"): `record_sql`
    /// (the observed-delta upsert, which reads the target table's
    /// PRE-write state) runs first, then every statement in `write_group`
    /// — all inside one `duckdb::Transaction`, so either both the delta
    /// record and the write commit, or neither does. `ensure_sql` (the
    /// idempotent `CREATE TABLE IF NOT EXISTS`) runs first, outside that
    /// transaction, same precedent as `fold_ledger_delta`'s own `ensure_sql`
    /// handling. `Transaction`'s default `DropBehavior::Rollback` means a
    /// failure anywhere in `record_sql` or `write_group` rolls back every
    /// statement already applied in this call — a failed write never leaves
    /// a delta row behind (the record and the write share one commit
    /// point), and a failed record never lets the write proceed.
    async fn execute_conditional_write_and_record_observed_delta(
        &self,
        ensure_sql: &str,
        write_group: &StatementGroup,
        record_sql: &str,
    ) -> Result<(), BackendError> {
        let ensure_sql = ensure_sql.to_string();
        let mut statements: Vec<String> = vec![record_sql.to_string()];
        statements.extend(write_group.statements.iter().map(|s| s.sql.clone()));
        let connection = Arc::clone(&self.connection);

        tokio::task::spawn_blocking(move || {
            let mut conn = connection.lock().expect("DuckDB connection mutex poisoned");

            conn.execute(&ensure_sql, [])
                .map_err(|e| BackendError::execution_failed("observed_delta", e.to_string()))?;

            let tx = conn
                .transaction()
                .map_err(|e| BackendError::execution_failed("observed_delta", e.to_string()))?;
            for sql in &statements {
                tx.execute(sql, [])
                    .map_err(|e| BackendError::execution_failed("observed_delta", e.to_string()))?;
            }
            tx.commit()
                .map_err(|e| BackendError::execution_failed("observed_delta", e.to_string()))?;
            Ok(())
        })
        .await
        .map_err(|e| BackendError::Other(e.into()))?
    }

    /// Real transactional override (F3, `docs/plans/20260715-composed-axes-
    /// conditional-maintenance.md` Phase F3; `docs/specs/sources.md`
    /// §"The fingerprint sidecar" — "Transactionality"): every statement in
    /// `write_group`, then `refresh_sql`, then `gc_sql` all run inside one
    /// `duckdb::Transaction`, so either the write and the sidecar's
    /// digest-refresh all commit, or none of them do. `ensure_sql` (the
    /// idempotent `CREATE TABLE IF NOT EXISTS`) runs first, outside that
    /// transaction — same precedent as `fold_ledger_delta`'s and
    /// `execute_conditional_write_and_record_observed_delta`'s own
    /// `ensure_sql` handling. `Transaction`'s default
    /// `DropBehavior::Rollback` means a failure anywhere in `write_group`,
    /// `refresh_sql`, or `gc_sql` rolls back every statement already
    /// applied in this call — a failed write never leaves a refreshed
    /// sidecar digest behind (the write and the refresh share one commit
    /// point).
    async fn execute_write_and_refresh_fingerprint_sidecar(
        &self,
        ensure_sql: &str,
        write_group: &StatementGroup,
        refresh_sql: &str,
        gc_sql: &str,
    ) -> Result<(), BackendError> {
        let ensure_sql = ensure_sql.to_string();
        let mut statements: Vec<String> = write_group
            .statements
            .iter()
            .map(|s| s.sql.clone())
            .collect();
        statements.push(refresh_sql.to_string());
        statements.push(gc_sql.to_string());
        let connection = Arc::clone(&self.connection);

        tokio::task::spawn_blocking(move || {
            let mut conn = connection.lock().expect("DuckDB connection mutex poisoned");

            conn.execute(&ensure_sql, []).map_err(|e| {
                BackendError::execution_failed("fingerprint_sidecar", e.to_string())
            })?;

            let tx = conn.transaction().map_err(|e| {
                BackendError::execution_failed("fingerprint_sidecar", e.to_string())
            })?;
            for sql in &statements {
                tx.execute(sql, []).map_err(|e| {
                    BackendError::execution_failed("fingerprint_sidecar", e.to_string())
                })?;
            }
            tx.commit().map_err(|e| {
                BackendError::execution_failed("fingerprint_sidecar", e.to_string())
            })?;
            Ok(())
        })
        .await
        .map_err(|e| BackendError::Other(e.into()))?
    }
}

/// Whether a DuckDB error message reports a constraint violation (`PRIMARY
/// KEY`/`UNIQUE`) rather than some other execution failure. DuckDB's error
/// text for this case reliably contains "constraint" (e.g. "Constraint
/// Error: Duplicate key ... violates primary key constraint").
fn is_constraint_violation(message: &str) -> bool {
    message.to_lowercase().contains("constraint")
}

#[cfg(test)]
mod tests {
    use super::*;
    use smelt_backend::Materialization;
    use tempfile::TempDir;

    const GIB: u64 = 1024 * 1024 * 1024;

    // ── default_memory_limit_bytes: min(50% RAM, RAM-20GiB), floored at 40% ──────

    #[test]
    fn default_memory_limit_60gib_uses_50pct_cap() {
        // 60 GiB: min(30, 40) = 30 GiB (the 50% cap), above the 24 GiB floor.
        assert_eq!(default_memory_limit_bytes(60 * GIB), 30 * GIB);
    }

    #[test]
    fn default_memory_limit_36gib_uses_minus20_term() {
        // 36 GiB: min(18, 16) = 16 GiB (RAM − 20 GiB), above the 14.4 GiB floor.
        assert_eq!(default_memory_limit_bytes(36 * GIB), 16 * GIB);
    }

    #[test]
    fn default_memory_limit_24gib_hits_40pct_floor() {
        // 24 GiB: min(12, 4) = 4 GiB → floored to 40% = 9.6 GiB so small hosts stay usable.
        assert_eq!(default_memory_limit_bytes(24 * GIB), 24 * GIB * 4 / 10);
    }

    #[test]
    fn default_memory_limit_128gib_uses_50pct_cap() {
        // 128 GiB: min(64, 108) = 64 GiB (the 50% cap).
        assert_eq!(default_memory_limit_bytes(128 * GIB), 64 * GIB);
    }

    // ── resolve_duckdb_settings: inject defaults, never override the user ─────────

    fn db_path() -> std::path::PathBuf {
        std::path::Path::new("/proj/target/dev.duckdb").to_path_buf()
    }

    #[test]
    fn resolve_injects_defaults_when_absent() {
        let s = resolve_duckdb_settings(None, Some(60 * GIB), &db_path());
        assert_eq!(s.get("memory_limit").map(String::as_str), Some("30720MiB")); // 30 GiB
        assert_eq!(
            s.get("temp_directory").map(String::as_str),
            Some("/proj/target/.smelt-duckdb-tmp")
        );
        assert!(!s.contains_key("threads"), "threads must be left untouched");
    }

    #[test]
    fn resolve_respects_user_memory_limit() {
        let mut user = BTreeMap::new();
        user.insert("memory_limit".to_string(), "4GB".to_string());
        let s = resolve_duckdb_settings(Some(&user), Some(60 * GIB), &db_path());
        assert_eq!(
            s.get("memory_limit").map(String::as_str),
            Some("4GB"),
            "explicit memory_limit must never be overridden"
        );
    }

    #[test]
    fn resolve_respects_user_temp_directory() {
        let mut user = BTreeMap::new();
        user.insert("temp_directory".to_string(), "/mnt/fast/tmp".to_string());
        let s = resolve_duckdb_settings(Some(&user), Some(60 * GIB), &db_path());
        assert_eq!(
            s.get("temp_directory").map(String::as_str),
            Some("/mnt/fast/tmp")
        );
    }

    #[test]
    fn resolve_no_ram_skips_memory_limit_but_sets_temp_dir() {
        let s = resolve_duckdb_settings(None, None, &db_path());
        assert!(
            !s.contains_key("memory_limit"),
            "no RAM info → no memory_limit default; DuckDB's own default stands"
        );
        assert_eq!(
            s.get("temp_directory").map(String::as_str),
            Some("/proj/target/.smelt-duckdb-tmp")
        );
    }

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

    /// `create_materialized_view_as` has no DuckDB override, so it falls
    /// through to the `Backend` trait's provided default
    /// (`crates/smelt-backend/src/lib.rs`), which reports that this backend
    /// has no native incremental-view maintenance — DuckDB's
    /// `supports_native_ivm` is `false`
    /// (`docs/specs/materialized_view.md` §"No silent fallback" item 1).
    #[tokio::test]
    async fn create_materialized_view_as_default_errors_no_native_ivm() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.duckdb");
        let backend = DuckDbBackend::new(&db_path, "main").await.unwrap();

        let err = backend
            .create_materialized_view_as("main", "mv", "SELECT 1 as id")
            .await
            .unwrap_err();

        let message = err.to_string();
        assert!(
            message.contains("no native incremental-view maintenance"),
            "unexpected error message: {message}"
        );
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
        assert!(!caps.supports_native_ivm);
        assert!(!caps.supports_retraction);
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
                &[],
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
                &[],
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
        let partition = smelt_backend::PartitionRange {
            column: "dt".to_string(),
            start: "2024-01-01".to_string(),
            end: "2024-01-02".to_string(),
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

    // ── delete_and_insert_transactional: per-chunk transaction boundary ─────────
    // (`incremental_shapes.md` §"First-run and backfill": "Each chunk's
    // DELETE+INSERT is one backend transaction. INSERT failure rolls back
    // the chunk's DELETE.")

    #[tokio::test]
    async fn test_delete_and_insert_transactional_commits_on_success() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.duckdb");
        let backend = DuckDbBackend::new(&db_path, "main").await.unwrap();

        backend
            .execute_sql(
                "CREATE TABLE main.daily AS SELECT * FROM (VALUES \
                 ('2024-01-01', 10), ('2024-01-02', 30)) AS t(dt, val)",
            )
            .await
            .unwrap();

        let partition = smelt_backend::PartitionRange {
            column: "dt".to_string(),
            start: "2024-01-01".to_string(),
            end: "2024-01-02".to_string(),
        };

        backend
            .delete_and_insert_transactional(
                "main",
                "daily",
                &partition,
                "SELECT '2024-01-01' as dt, 999 as val",
            )
            .await
            .unwrap();

        let count = backend.get_row_count("main", "daily").await.unwrap();
        assert_eq!(count, 2, "delete removed 1 row, insert added 1 row back");

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
        assert_eq!(val, 999, "the replacement row from the INSERT is present");
    }

    #[tokio::test]
    async fn test_delete_and_insert_transactional_rolls_back_delete_on_insert_failure() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.duckdb");
        let backend = DuckDbBackend::new(&db_path, "main").await.unwrap();

        backend
            .execute_sql("CREATE TABLE main.daily (dt VARCHAR, val INTEGER)")
            .await
            .unwrap();
        backend
            .execute_sql(
                "INSERT INTO main.daily VALUES ('2024-01-01', 10), ('2024-01-01', 20), \
                 ('2024-01-02', 30)",
            )
            .await
            .unwrap();

        let before_count = backend.get_row_count("main", "daily").await.unwrap();
        assert_eq!(before_count, 3);

        let partition = smelt_backend::PartitionRange {
            column: "dt".to_string(),
            start: "2024-01-01".to_string(),
            end: "2024-01-02".to_string(),
        };

        // The INSERT SELECT references a column that doesn't exist in
        // `main.daily`'s schema (dt, val) — this fails at INSERT time, after
        // the DELETE has already run inside the same transaction.
        let result = backend
            .delete_and_insert_transactional(
                "main",
                "daily",
                &partition,
                "SELECT '2024-01-01' as dt, 999 as val, 'bogus' as nonexistent_column",
            )
            .await;

        assert!(
            result.is_err(),
            "an INSERT into a mismatched schema should fail"
        );

        // The DELETE must have been rolled back — the table state must equal
        // what it was before this (failed) attempt, not "deleted with no
        // replacement rows".
        let after_count = backend.get_row_count("main", "daily").await.unwrap();
        assert_eq!(
            after_count, before_count,
            "a failed INSERT must roll back the paired DELETE"
        );

        let jan1_count: usize = {
            let rows = backend
                .execute_sql("SELECT val FROM main.daily WHERE dt = '2024-01-01' ORDER BY val")
                .await
                .unwrap();
            rows.iter().map(|b| b.num_rows()).sum()
        };
        assert_eq!(
            jan1_count, 2,
            "the two 2024-01-01 rows deleted mid-transaction must be restored"
        );
    }

    // ── fold_ledger_delta: warehouse-resident per-delta ledger (MP12) ────
    // (`docs/specs/incremental_models.md` §Constraints "Never fold a delta
    // already reflected in the state" — the DuckDB override must run the
    // ledger insert and the paired fold action as one transaction.)

    fn ledger_sql(
        model: &str,
        delta_id: &str,
        action_sql: &str,
    ) -> (String, String, String, String) {
        let ensure_sql = smelt_state::ddl_duckdb::generate_ledger_table_ddl("main");
        let insert_sql = smelt_state::ddl_duckdb::generate_ledger_insert_sql(
            "main",
            model,
            "{*}",
            "smelt.events",
            delta_id,
            "2026-01-01",
            "2026-01-02",
        );
        let exists_sql = smelt_state::ddl_duckdb::generate_ledger_exists_sql(
            "main",
            model,
            "{*}",
            "smelt.events",
            delta_id,
        );
        (ensure_sql, insert_sql, exists_sql, action_sql.to_string())
    }

    #[tokio::test]
    async fn test_fold_ledger_delta_commits_ledger_and_action_together() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.duckdb");
        let backend = DuckDbBackend::new(&db_path, "main").await.unwrap();

        backend
            .execute_sql("CREATE TABLE main.device_stats (n INTEGER)")
            .await
            .unwrap();

        let (ensure_sql, insert_sql, exists_sql, action_sql) = ledger_sql(
            "device_stats",
            "2026-01-01",
            "INSERT INTO main.device_stats VALUES (1)",
        );

        backend
            .fold_ledger_delta(&ensure_sql, &insert_sql, &exists_sql, &action_sql)
            .await
            .expect("first fold commits");

        let count = backend.get_row_count("main", "device_stats").await.unwrap();
        assert_eq!(count, 1, "the paired action ran and committed");
    }

    #[tokio::test]
    async fn test_fold_ledger_delta_refuses_repeat_and_never_reruns_action() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.duckdb");
        let backend = DuckDbBackend::new(&db_path, "main").await.unwrap();

        backend
            .execute_sql("CREATE TABLE main.device_stats (n INTEGER)")
            .await
            .unwrap();

        let (ensure_sql, insert_sql, exists_sql, action_sql) = ledger_sql(
            "device_stats",
            "2026-01-01",
            "INSERT INTO main.device_stats VALUES (1)",
        );

        backend
            .fold_ledger_delta(&ensure_sql, &insert_sql, &exists_sql, &action_sql)
            .await
            .expect("first fold commits");

        // A repeat of the exact same delta identity — the ledger's PRIMARY
        // KEY refuses inside the transaction, and the paired action must
        // not run a second time (no check-then-act race across the write).
        let result = backend
            .fold_ledger_delta(&ensure_sql, &insert_sql, &exists_sql, &action_sql)
            .await;

        assert!(
            matches!(result, Err(BackendError::AlreadyReflected { .. })),
            "repeat delta must surface AlreadyReflected, got: {:?}",
            result
        );

        let count = backend.get_row_count("main", "device_stats").await.unwrap();
        assert_eq!(
            count, 1,
            "the paired action must not run a second time once the ledger insert was refused"
        );
    }

    #[tokio::test]
    async fn test_fold_ledger_delta_distinct_deltas_both_apply() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.duckdb");
        let backend = DuckDbBackend::new(&db_path, "main").await.unwrap();

        backend
            .execute_sql("CREATE TABLE main.device_stats (n INTEGER)")
            .await
            .unwrap();

        let (ensure_sql, insert_sql, exists_sql, action_sql) = ledger_sql(
            "device_stats",
            "2026-01-01",
            "INSERT INTO main.device_stats VALUES (1)",
        );
        backend
            .fold_ledger_delta(&ensure_sql, &insert_sql, &exists_sql, &action_sql)
            .await
            .expect("first delta folds");

        let (ensure_sql, insert_sql, exists_sql, action_sql) = ledger_sql(
            "device_stats",
            "2026-01-02",
            "INSERT INTO main.device_stats VALUES (2)",
        );
        backend
            .fold_ledger_delta(&ensure_sql, &insert_sql, &exists_sql, &action_sql)
            .await
            .expect("a distinct delta identity is not refused");

        let count = backend.get_row_count("main", "device_stats").await.unwrap();
        assert_eq!(count, 2, "both distinct deltas' actions ran");
    }

    // ── execute_conditional_write_and_record_observed_delta (T5) ───────

    #[tokio::test]
    async fn test_record_observed_delta_commits_write_and_record_together() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.duckdb");
        let backend = DuckDbBackend::new(&db_path, "main").await.unwrap();

        backend
            .execute_sql("CREATE TABLE main.device_stats (id INTEGER)")
            .await
            .unwrap();

        let ensure_sql = smelt_state::ddl_duckdb::generate_observed_delta_table_ddl("main");
        let write_group = StatementGroup {
            statements: vec![smelt_backend::MaintenanceStatement {
                sql: "INSERT INTO main.device_stats VALUES (1)".to_string(),
            }],
            transactional: false,
        };
        let record_sql = smelt_state::ddl_duckdb::generate_observed_delta_upsert_sql(
            "main",
            "device_stats",
            "2026-01-01",
            "2026-01-02",
            "SELECT '1' AS delta_key, NULL AS delta_partition",
        );

        backend
            .execute_conditional_write_and_record_observed_delta(
                &ensure_sql,
                &write_group,
                &record_sql,
            )
            .await
            .expect("write + record commits together");

        let count = backend.get_row_count("main", "device_stats").await.unwrap();
        assert_eq!(count, 1, "the write ran and committed");

        let rows = backend
            .execute_sql("SELECT changed_keys, partitions FROM main._smelt_observed_delta")
            .await
            .unwrap();
        let total_rows: usize = rows.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total_rows, 1, "exactly one observed-delta row recorded");
    }

    #[tokio::test]
    async fn test_record_observed_delta_rolls_back_record_on_write_failure() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.duckdb");
        let backend = DuckDbBackend::new(&db_path, "main").await.unwrap();

        backend
            .execute_sql("CREATE TABLE main.device_stats (id INTEGER)")
            .await
            .unwrap();

        let ensure_sql = smelt_state::ddl_duckdb::generate_observed_delta_table_ddl("main");
        // A write statement that fails (references a nonexistent table) —
        // the record must never land.
        let write_group = StatementGroup {
            statements: vec![smelt_backend::MaintenanceStatement {
                sql: "INSERT INTO main.does_not_exist VALUES (1)".to_string(),
            }],
            transactional: false,
        };
        let record_sql = smelt_state::ddl_duckdb::generate_observed_delta_upsert_sql(
            "main",
            "device_stats",
            "2026-01-01",
            "2026-01-02",
            "SELECT '1' AS delta_key, NULL AS delta_partition",
        );

        let result = backend
            .execute_conditional_write_and_record_observed_delta(
                &ensure_sql,
                &write_group,
                &record_sql,
            )
            .await;
        assert!(result.is_err(), "the failed write must surface an error");

        let rows = backend
            .execute_sql("SELECT * FROM main._smelt_observed_delta")
            .await
            .unwrap();
        let total_rows: usize = rows.iter().map(|b| b.num_rows()).sum();
        assert_eq!(
            total_rows, 0,
            "a failed write must leave no observed-delta row behind"
        );
    }

    // ── execute_write_and_refresh_fingerprint_sidecar (F3) ──────────────

    #[tokio::test]
    async fn test_fingerprint_sidecar_commits_write_and_refresh_together() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.duckdb");
        let backend = DuckDbBackend::new(&db_path, "main").await.unwrap();

        backend
            .execute_sql("CREATE TABLE main.device_stats (id INTEGER)")
            .await
            .unwrap();

        let ensure_sql = smelt_state::ddl_duckdb::generate_fingerprint_sidecar_table_ddl("main");
        let write_group = StatementGroup {
            statements: vec![smelt_backend::MaintenanceStatement {
                sql: "INSERT INTO main.device_stats VALUES (1)".to_string(),
            }],
            transactional: false,
        };
        let digest_select = "SELECT '1' AS delta_key, 'digest-1' AS delta_digest";
        let refresh_sql = smelt_state::ddl_duckdb::generate_fingerprint_sidecar_refresh_sql(
            "main",
            "smelt.sources.dim_users",
            "cols:name",
            "v1:cols:name:sha256:deadbeef",
            digest_select,
        );
        let gc_sql = smelt_state::ddl_duckdb::generate_fingerprint_sidecar_gc_sql(
            "main",
            "smelt.sources.dim_users",
            "cols:name",
            digest_select,
        );

        backend
            .execute_write_and_refresh_fingerprint_sidecar(
                &ensure_sql,
                &write_group,
                &refresh_sql,
                &gc_sql,
            )
            .await
            .expect("write + sidecar refresh commits together");

        let count = backend.get_row_count("main", "device_stats").await.unwrap();
        assert_eq!(count, 1, "the write ran and committed");

        let rows = backend
            .execute_sql("SELECT source_key, digest FROM main._smelt_fingerprint_sidecar")
            .await
            .unwrap();
        let total_rows: usize = rows.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total_rows, 1, "exactly one sidecar row refreshed");
    }

    #[tokio::test]
    async fn test_fingerprint_sidecar_rolls_back_refresh_on_write_failure() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.duckdb");
        let backend = DuckDbBackend::new(&db_path, "main").await.unwrap();

        backend
            .execute_sql("CREATE TABLE main.device_stats (id INTEGER)")
            .await
            .unwrap();

        let ensure_sql = smelt_state::ddl_duckdb::generate_fingerprint_sidecar_table_ddl("main");
        // A write statement that fails (references a nonexistent table) —
        // the sidecar refresh must never land.
        let write_group = StatementGroup {
            statements: vec![smelt_backend::MaintenanceStatement {
                sql: "INSERT INTO main.does_not_exist VALUES (1)".to_string(),
            }],
            transactional: false,
        };
        let digest_select = "SELECT '1' AS delta_key, 'digest-1' AS delta_digest";
        let refresh_sql = smelt_state::ddl_duckdb::generate_fingerprint_sidecar_refresh_sql(
            "main",
            "smelt.sources.dim_users",
            "cols:name",
            "v1:cols:name:sha256:deadbeef",
            digest_select,
        );
        let gc_sql = smelt_state::ddl_duckdb::generate_fingerprint_sidecar_gc_sql(
            "main",
            "smelt.sources.dim_users",
            "cols:name",
            digest_select,
        );

        let result = backend
            .execute_write_and_refresh_fingerprint_sidecar(
                &ensure_sql,
                &write_group,
                &refresh_sql,
                &gc_sql,
            )
            .await;
        assert!(result.is_err(), "the failed write must surface an error");

        let rows = backend
            .execute_sql("SELECT * FROM main._smelt_fingerprint_sidecar")
            .await
            .unwrap();
        let total_rows: usize = rows.iter().map(|b| b.num_rows()).sum();
        assert_eq!(
            total_rows, 0,
            "a failed write must leave no sidecar digest behind"
        );
    }

    /// `resolve_strategy` is no longer a dispatching function — it always
    /// returns DeleteInsert. MERGE is the physical primitive of the
    /// `cumulative_aggregate` materialization and is not selected through
    /// the IncrementalStrategy enum.
    #[tokio::test]
    async fn test_resolve_strategy_always_delete_insert() {
        use smelt_backend::PartitionGrainConfig;
        use smelt_backend::PartitionGrainSafetyOverrides;

        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.duckdb");
        let backend = DuckDbBackend::new(&db_path, "main").await.unwrap();

        let config_with_key = PartitionGrainConfig {
            unique_key: vec!["id".to_string()],
            nondeterministic_columns_retired: (),
            safety_overrides: PartitionGrainSafetyOverrides::default(),
        };
        assert_eq!(
            backend.resolve_strategy(&config_with_key),
            smelt_backend::IncrementalStrategy::DeleteInsert,
        );

        let config_without_key = PartitionGrainConfig {
            unique_key: vec![],
            nondeterministic_columns_retired: (),
            safety_overrides: PartitionGrainSafetyOverrides::default(),
        };
        assert_eq!(
            backend.resolve_strategy(&config_without_key),
            smelt_backend::IncrementalStrategy::DeleteInsert,
        );
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
