//! Backend trait and types for smelt execution engines.
//!
//! This crate defines the abstract interface that all smelt backends must implement,
//! enabling multi-backend support (DuckDB, Spark, etc.).

mod error;
mod types;

pub use error::BackendError;
pub use smelt_core::config::{
    Granularity, IncrementalStrategy, PartitionGrainConfig, PartitionGrainSafetyOverrides,
};
pub use smelt_dialect::{BackendCapabilities, SqlDialect};
pub use smelt_logical::maintenance::emit::{
    emit_column_scoped_merge, emit_delete_insert, partition_literal, MaintenanceDialect,
    MaintenanceStatement, Region, StatementGroup,
};
pub use smelt_logical::PartitionAxis;
pub use types::{
    ExecutionResult, Materialization, MaterializationStrategy, PartitionRange, PartitionSpec,
};

use arrow::array::RecordBatch;
use arrow::datatypes::SchemaRef;
use async_trait::async_trait;

/// Map a backend's [`SqlDialect`] to the [`MaintenanceDialect`] the
/// single-owner emitters key their dialect-specific variants on. The region
/// `DELETE`+`INSERT` family is dialect-invariant; the whole-row `MERGE`
/// families are not — GoogleSQL accepts neither `UPDATE SET *` nor
/// `INSERT *`, so `BigQuery` selects the spelled-out arms.
pub fn maintenance_dialect(dialect: SqlDialect) -> MaintenanceDialect {
    match dialect {
        SqlDialect::DuckDB => MaintenanceDialect::DuckDb,
        SqlDialect::SparkSQL => MaintenanceDialect::Spark,
        SqlDialect::BigQuery => MaintenanceDialect::BigQuery,
    }
}

/// Build the transactional region `DELETE`+`INSERT` [`StatementGroup`] for
/// an incremental batch — the single call site every `Backend` impl's
/// `delete_and_insert_transactional` routes through, so the emitted text is
/// the only text a backend ever executes for this family
/// (`docs/specs/incremental_models.md` §"Statement emission (single owner)").
fn build_delete_insert_group(
    schema: &str,
    name: &str,
    partition: &PartitionRange,
    sql: &str,
    dialect: SqlDialect,
) -> Result<StatementGroup, String> {
    let table_name = format!("{schema}.{name}");
    let region = Region::for_axis(partition.axis, &partition.start, &partition.end)?;
    Ok(emit_delete_insert(
        &table_name,
        &partition.column,
        &region,
        sql,
        maintenance_dialect(dialect),
    ))
}

/// Refuse a whole-row `MERGE` whose dialect needs an explicit column list and
/// was not given one.
///
/// DuckDB and Spark spell the matched arm `UPDATE SET *` and never read
/// `columns`; GoogleSQL has no star form, so an empty list there would emit a
/// syntactically valid `MERGE` whose matched arm assigns nothing — rows would
/// silently stop being updated. Fail-loud discipline (`architecture.md`
/// §"Fail-loud discipline") makes that an error naming the model, not a
/// degraded write.
pub fn require_merge_columns(
    dialect: SqlDialect,
    schema: &str,
    table: &str,
    columns: &[String],
) -> Result<(), BackendError> {
    if matches!(maintenance_dialect(dialect), MaintenanceDialect::BigQuery) && columns.is_empty() {
        return Err(BackendError::execution_failed(
            format!("{schema}.{table}"),
            "column-scoped MERGE on BigQuery needs the model's output column list, and none was \
             resolved — GoogleSQL has no `UPDATE SET *`, so the emitted matched arm would assign \
             no columns. This usually means the model's output columns are not statically \
             resolvable (e.g. a surviving `SELECT *`); name the columns in the model's projection."
                .to_string(),
        ));
    }
    Ok(())
}

/// Build the column-scoped `MERGE` [`StatementGroup`] for `Backend::
/// merge_into`'s default implementation — the single call site every
/// `Backend` impl routes through unless it overrides `merge_into` itself
/// (`docs/specs/incremental_models.md` §"Statement emission (single owner)").
fn build_column_scoped_merge_group(
    schema: &str,
    table: &str,
    source_sql: &str,
    unique_key: &[String],
    columns: &[String],
    dialect: SqlDialect,
) -> StatementGroup {
    let table_name = format!("{schema}.{table}");
    emit_column_scoped_merge(
        &table_name,
        unique_key,
        source_sql,
        columns,
        maintenance_dialect(dialect),
    )
}

/// The shared body of [`Backend::execute_model`]'s provided default: drop
/// whichever materialization kind the model isn't (in case it changed since
/// the last run), create the requested kind, then report row count and an
/// optional preview.
///
/// This is a free function, not just code inlined into the trait's default
/// method, because Rust gives an overriding trait impl no way to call back
/// into the default implementation it replaced — there is no `super`.
/// `BigQueryBackend::execute_model` needs to run one extra step first
/// (dropping a leftover materialized view before the ordinary drop/create
/// pair — `docs/specs/materialized_view.md`, reverse-flip cleanup) and then
/// run exactly this behaviour; factoring it out here is what lets both the
/// trait's own default and that override share one implementation instead
/// of the override duplicating this body.
pub async fn execute_model_default(
    backend: &(impl Backend + ?Sized),
    schema: &str,
    name: &str,
    sql: &str,
    materialization: Materialization,
    show_preview: bool,
) -> Result<ExecutionResult, BackendError> {
    let start = std::time::Instant::now();

    match materialization {
        Materialization::Table => {
            // Drop both view and table in case the materialization type changed
            backend.drop_view_if_exists(schema, name).await?;
            backend.drop_table_if_exists(schema, name).await?;
            backend.create_table_as(schema, name, sql).await?;
        }
        Materialization::View => {
            // Drop both table and view in case the materialization type changed
            backend.drop_table_if_exists(schema, name).await?;
            backend.drop_view_if_exists(schema, name).await?;
            backend.create_view_as(schema, name, sql).await?;
        }
    }

    let duration = start.elapsed();
    let row_count = backend.get_row_count(schema, name).await?;

    let preview = if show_preview {
        Some(backend.get_preview(schema, name, 10).await?)
    } else {
        None
    };

    Ok(ExecutionResult {
        model_name: name.to_string(),
        duration,
        row_count,
        preview,
    })
}

/// Abstract interface for smelt execution backends.
///
/// Backends are responsible for:
/// - Executing SQL queries
/// - Creating tables and views
/// - Validating source tables exist
/// - Reporting their SQL dialect and capabilities
#[async_trait]
pub trait Backend: Send + Sync {
    /// Execute a SQL query and return results.
    async fn execute_sql(&self, sql: &str) -> Result<Vec<RecordBatch>, BackendError>;

    /// Create a table from a SQL query.
    async fn create_table_as(
        &self,
        schema: &str,
        name: &str,
        sql: &str,
    ) -> Result<(), BackendError>;

    /// Create a view from a SQL query.
    async fn create_view_as(&self, schema: &str, name: &str, sql: &str)
        -> Result<(), BackendError>;

    /// Create (or replace) an engine-maintained materialized view from a SQL
    /// query — the `refresh: materialized_view` delegation target
    /// (`docs/specs/materialized_view.md`). Freshness is owned by the
    /// backend's native incremental-view-maintenance runtime, not by smelt.
    ///
    /// This is a **provided** method with an erroring default, not a
    /// required one: `supports_native_ivm` is `false` for every backend
    /// today (`docs/specs/multi_backend.md`), and a required method would
    /// force a stub implementation into every test-only mock `Backend`
    /// (~14 of them) for a capability none of them exercise. A backend that
    /// actually has native IVM (e.g. BigQuery) overrides this; every other
    /// backend inherits the default, which reports — via
    /// [`BackendError::UnsupportedFeature`] — that it has no native IVM,
    /// matching `docs/specs/materialized_view.md` §"No silent fallback"
    /// item 1. Callers should not normally reach this default: the
    /// `supports_native_ivm` capability gate in `smelt-runtime`'s compiler
    /// refuses the model before any backend call is made; this default is a
    /// second line of defense, not the primary enforcement point.
    async fn create_materialized_view_as(
        &self,
        schema: &str,
        name: &str,
        sql: &str,
    ) -> Result<(), BackendError> {
        let _ = sql;
        Err(BackendError::unsupported(
            self.dialect().name(),
            format!(
                "refresh: materialized_view for '{schema}.{name}' — this backend has no \
                 native incremental-view maintenance"
            ),
        ))
    }

    /// Drop a table if it exists.
    async fn drop_table_if_exists(&self, schema: &str, name: &str) -> Result<(), BackendError>;

    /// Drop a view if it exists.
    async fn drop_view_if_exists(&self, schema: &str, name: &str) -> Result<(), BackendError>;

    /// Get the row count of a table or view.
    async fn get_row_count(&self, schema: &str, name: &str) -> Result<usize, BackendError>;

    /// Get a preview of a table or view (first N rows).
    async fn get_preview(
        &self,
        schema: &str,
        name: &str,
        limit: usize,
    ) -> Result<Vec<RecordBatch>, BackendError>;

    /// Check if a table exists.
    async fn table_exists(&self, schema: &str, name: &str) -> Result<bool, BackendError>;

    /// Ensure a schema exists, creating it if necessary.
    async fn ensure_schema(&self, schema: &str) -> Result<(), BackendError>;

    /// Get the SQL dialect this backend uses.
    fn dialect(&self) -> SqlDialect;

    /// Get the capabilities of this backend.
    fn capabilities(&self) -> BackendCapabilities;

    /// Load Arrow record batches into a new table at `schema.name`.
    ///
    /// Drops any existing table or view with the same name first (like seed loading does).
    /// Returns an error if a non-nullable column (Arrow `Field::nullable == false`) contains
    /// NULL values.
    ///
    /// Supported Arrow types (matching the seed type set from `seeds.md §"Type inference"`):
    /// - `DataType::Boolean` → `BOOLEAN`
    /// - `DataType::Int32` → `INTEGER`
    /// - `DataType::Int64` → `BIGINT`
    /// - `DataType::Decimal128(p, s)` → `DECIMAL(p, s)` with p ≤ 18 and s ≤ 4
    /// - `DataType::Float64` → `DOUBLE`
    /// - `DataType::Date32` → `DATE`
    /// - `DataType::Timestamp(TimeUnit::Microsecond, None)` → `TIMESTAMP` (no TZ)
    /// - `DataType::Utf8` → `VARCHAR`
    async fn load_table(
        &self,
        schema: &str,
        name: &str,
        arrow_schema: SchemaRef,
        batches: Vec<RecordBatch>,
    ) -> Result<(), BackendError>;

    /// Get the filesystem path where a model's output is materialized.
    /// Returns None for database backends (DuckDB), Some(path) for file-based backends (Spark).
    fn materialized_path(&self, schema: &str, name: &str) -> Option<std::path::PathBuf> {
        let _ = (schema, name);
        None
    }

    /// Execute a model (drop + create as table or view).
    ///
    /// This is a convenience method that combines drop + create operations.
    /// The body lives in the free function [`execute_model_default`] rather
    /// than inline here so a backend that overrides this method can still
    /// run it after doing its own extra step first — see that function's
    /// doc comment for why (a trait override has no way to call back into
    /// the default it replaced). `BigQueryBackend::execute_model` is the
    /// first such override, cleaning up a leftover materialized view before
    /// falling through to this behaviour (`docs/specs/materialized_view.md`).
    async fn execute_model(
        &self,
        schema: &str,
        name: &str,
        sql: &str,
        materialization: Materialization,
        show_preview: bool,
    ) -> Result<ExecutionResult, BackendError> {
        execute_model_default(self, schema, name, sql, materialization, show_preview).await
    }

    /// Execute a model with incremental materialization support.
    ///
    /// Dispatches to the appropriate strategy (DELETE+INSERT, MERGE, APPEND, INSERT OVERWRITE).
    /// Thin call into [`Backend::execute_model_incremental_with_bookkeeping`] with no
    /// bookkeeping statements — kept so every existing caller that has no reset/ledger
    /// record to attach is unaffected.
    async fn execute_model_incremental(
        &self,
        schema: &str,
        name: &str,
        sql: &str,
        materialization: Materialization,
        strategy: MaterializationStrategy,
        show_preview: bool,
    ) -> Result<ExecutionResult, BackendError> {
        self.execute_model_incremental_with_bookkeeping(
            schema,
            name,
            sql,
            materialization,
            strategy,
            show_preview,
            &[],
            &[],
        )
        .await
    }

    /// Execute a model with incremental materialization support, running
    /// `ensure_sqls` (idempotent DDL) and `pre_write_sqls` (bookkeeping
    /// records — e.g. the reconciliation ledger's region-recompute reset,
    /// `docs/specs/incremental_models.md` §"The reconciliation ledger")
    /// alongside the write. For the `(Table, Incremental{DeleteInsert})`
    /// case on an existing table, the delete+insert group is built via the
    /// shared emitter and routed through
    /// [`Backend::execute_write_with_bookkeeping`] so the reset and the
    /// write share one backend transaction where the backend can provide
    /// one (DuckDB does). Every other arm has no write group to attach
    /// bookkeeping to in the same transaction — it runs `ensure_sqls`/
    /// `pre_write_sqls` as standalone statements first, then falls through
    /// to today's logic, unchanged.
    ///
    /// No DuckDB override is needed: `execute_write_with_bookkeeping`
    /// already has DuckDB's real transactional override.
    #[allow(clippy::too_many_arguments)]
    async fn execute_model_incremental_with_bookkeeping(
        &self,
        schema: &str,
        name: &str,
        sql: &str,
        materialization: Materialization,
        strategy: MaterializationStrategy,
        show_preview: bool,
        ensure_sqls: &[String],
        pre_write_sqls: &[String],
    ) -> Result<ExecutionResult, BackendError> {
        let start = std::time::Instant::now();

        match (materialization, strategy) {
            (Materialization::View, _) => {
                // Views can't be incremental — full refresh
                self.execute_model(schema, name, sql, materialization, show_preview)
                    .await?;
                // Return early to avoid double row count/preview
                return Ok(ExecutionResult {
                    model_name: name.to_string(),
                    duration: start.elapsed(),
                    row_count: self.get_row_count(schema, name).await?,
                    preview: if show_preview {
                        Some(self.get_preview(schema, name, 10).await?)
                    } else {
                        None
                    },
                });
            }
            (Materialization::Table, MaterializationStrategy::FullRefresh) => {
                for ensure_sql in ensure_sqls {
                    self.execute_sql(ensure_sql).await?;
                }
                for pre_write_sql in pre_write_sqls {
                    self.execute_sql(pre_write_sql).await?;
                }
                self.drop_table_if_exists(schema, name).await?;
                self.create_table_as(schema, name, sql).await?;
            }
            (
                Materialization::Table,
                MaterializationStrategy::Incremental {
                    partition,
                    strategy: inc_strategy,
                    unique_key,
                },
            ) => {
                let table_exists = self.table_exists(schema, name).await?;

                if !table_exists {
                    for ensure_sql in ensure_sqls {
                        self.execute_sql(ensure_sql).await?;
                    }
                    for pre_write_sql in pre_write_sqls {
                        self.execute_sql(pre_write_sql).await?;
                    }
                    self.create_table_as(schema, name, sql).await?;
                } else {
                    let _ = unique_key; // unused since the cumulative path owns merge_into
                    match inc_strategy {
                        IncrementalStrategy::DeleteInsert => {
                            let group = build_delete_insert_group(
                                schema,
                                name,
                                &partition,
                                sql,
                                self.dialect(),
                            )
                            .map_err(|message| BackendError::ConfigurationError { message })?;
                            self.execute_write_with_bookkeeping(
                                ensure_sqls,
                                pre_write_sqls,
                                &group,
                            )
                            .await?;
                        }
                    }
                }
            }
        }

        let duration = start.elapsed();
        let row_count = self.get_row_count(schema, name).await?;

        let preview = if show_preview {
            Some(self.get_preview(schema, name, 10).await?)
        } else {
            None
        };

        Ok(ExecutionResult {
            model_name: name.to_string(),
            duration,
            row_count,
            preview,
        })
    }

    /// Resolve the best incremental strategy for the given config.
    ///
    /// Default implementation: always returns `DeleteInsert`. MERGE is no
    /// longer an incremental strategy — it is the physical primitive of the
    /// `cumulative_aggregate` materialization (see
    /// `docs/specs/cumulative_aggregate.md`). The `unique_key` field on
    /// `PartitionGrainConfig` is reserved for backends that may want to use it
    /// for diagnostics or audit; it does not change strategy selection here.
    fn resolve_strategy(&self, _config: &PartitionGrainConfig) -> IncrementalStrategy {
        IncrementalStrategy::DeleteInsert
    }

    /// Delete rows in a half-open partition range `[start, end)`.
    ///
    /// Emits `DELETE FROM table WHERE column >= start AND column < end`.
    /// This form is both more efficient than an IN-list for large windows and
    /// is correct for any window size without enumerating individual partition
    /// values.
    async fn delete_partitions(
        &self,
        schema: &str,
        name: &str,
        partition: &PartitionRange,
    ) -> Result<(), BackendError>;

    /// Insert data from a SELECT query into an existing table.
    async fn insert_into_from_query(
        &self,
        schema: &str,
        name: &str,
        sql: &str,
    ) -> Result<(), BackendError>;

    /// Delete a partition range and insert the replacement rows as **one**
    /// backend transaction (`docs/specs/incremental_shapes.md` §"First-run and
    /// backfill" — "Each chunk's DELETE+INSERT is one backend transaction.
    /// INSERT failure rolls back the chunk's DELETE; earlier committed
    /// chunks do not roll back.").
    ///
    /// The statement text comes from `smelt_logical::maintenance::emit`
    /// (`docs/specs/incremental_models.md` §"Statement emission (single
    /// owner)") — this method builds the [`StatementGroup`] and hands it to
    /// [`Backend::execute_statement_group`], never authoring `DELETE`/
    /// `INSERT` text itself. Override `execute_statement_group`, not this
    /// method, to change how the group is executed (e.g. a real backend
    /// transaction).
    async fn delete_and_insert_transactional(
        &self,
        schema: &str,
        name: &str,
        partition: &PartitionRange,
        sql: &str,
    ) -> Result<(), BackendError> {
        let group = build_delete_insert_group(schema, name, partition, sql, self.dialect())
            .map_err(|message| BackendError::ConfigurationError { message })?;
        self.execute_statement_group(&group).await
    }

    /// Execute an emitted [`StatementGroup`] — the single point every
    /// `smelt_logical::maintenance::emit` consumer routes through
    /// (`docs/specs/incremental_models.md` §"Statement emission (single
    /// owner)"). Backends execute; they never author the statement text.
    ///
    /// Default implementation runs each statement sequentially via
    /// `execute_sql` — a best-effort, **non-atomic** fallback for any
    /// backend that does not override it (the same precedent as
    /// [`Backend::fold_ledger_delta`]'s default). A backend that can wrap
    /// a `group.transactional` group in a native transaction (DuckDB)
    /// should override this so a failure mid-group rolls back every
    /// statement already applied in this call.
    async fn execute_statement_group(&self, group: &StatementGroup) -> Result<(), BackendError> {
        for stmt in &group.statements {
            self.execute_sql(&stmt.sql).await?;
        }
        Ok(())
    }

    /// MERGE (upsert) rows using unique_key columns for matching.
    ///
    /// Matched rows are updated, unmatched rows are inserted. The statement
    /// text comes from `smelt_logical::maintenance::emit::
    /// emit_column_scoped_merge` (`docs/specs/incremental_models.md`
    /// §"Statement emission (single owner)") — this method builds the
    /// [`StatementGroup`] and hands it to [`Backend::execute_statement_group`],
    /// never authoring `MERGE` text itself. `source_sql` must project the
    /// full target row (see the emitter's doc comment for the full-row
    /// projection contract `UPDATE SET *` relies on).
    ///
    /// `columns` names that same full target row. It is inert on the dialects
    /// with a star form and **required** on BigQuery, whose GoogleSQL has
    /// none — passing an empty list there emits a `MERGE` whose matched arm
    /// updates nothing, so this method refuses rather than executing it.
    ///
    /// Default implementation routes through the shared emitter and
    /// `execute_statement_group`; a backend only needs to override this if
    /// it cannot express the emitted `MERGE` text at all (see
    /// [`BackendCapabilities::supports_column_scoped_merge`], read via
    /// [`Backend::capabilities`] — a genuine backend-capability gate, not a
    /// policy choice: a backend that cannot run a targeted `MERGE` must
    /// drop the technique from admission **at plan time**, never surface a
    /// runtime surprise after the plan already chose it).
    async fn merge_into(
        &self,
        schema: &str,
        table: &str,
        source_sql: &str,
        unique_key: &[String],
        columns: &[String],
    ) -> Result<(), BackendError> {
        let dialect = self.dialect();
        require_merge_columns(dialect, schema, table, columns)?;
        let group = build_column_scoped_merge_group(
            schema, table, source_sql, unique_key, columns, dialect,
        );
        self.execute_statement_group(&group).await
    }

    /// Replace partitions by inserting new data and removing old partition rows.
    ///
    /// Backends with native INSERT OVERWRITE use that; others emulate via
    /// DELETE (partition values derived from source) + INSERT.
    async fn insert_overwrite(
        &self,
        schema: &str,
        table: &str,
        sql: &str,
        partition: &PartitionRange,
    ) -> Result<(), BackendError>;

    /// Fold one delta identity into the warehouse-resident per-delta
    /// reconciliation ledger and run `action_sql` (a `CREATE TABLE ... AS`
    /// or `MERGE INTO` statement), refusing — without running `action_sql`
    /// — if the delta is already reflected
    /// (`docs/specs/incremental_models.md` §Constraints "Never fold a delta
    /// already reflected in the state").
    ///
    /// `ensure_sql` creates the ledger table if it does not already exist
    /// (idempotent DDL); `insert_sql` records the delta identity and
    /// violates the ledger table's `PRIMARY KEY` iff it is already present;
    /// `exists_sql` is a `SELECT`-based existence check for backends that
    /// cannot wrap `insert_sql` and `action_sql` in one native transaction.
    /// All four strings come from `smelt_state::ddl_duckdb` (or the
    /// matching Spark builder) — this trait does not depend on
    /// `smelt-state` itself, only executes the SQL text a caller with that
    /// dependency built.
    ///
    /// Default implementation is a best-effort, **non-atomic** fallback
    /// (`ensure_sql`, then `exists_sql`, then `insert_sql` + `action_sql`
    /// as separate statements) for any backend that does not override it —
    /// the same precedent as [`Backend::delete_and_insert_transactional`]'s
    /// default. A backend that can wrap `insert_sql` and `action_sql` in a
    /// native transaction (DuckDB) should override this so a repeat delta
    /// is refused by the ledger table's own constraint inside that
    /// transaction — no check-then-act race across the write.
    async fn fold_ledger_delta(
        &self,
        ensure_sql: &str,
        insert_sql: &str,
        exists_sql: &str,
        action_sql: &str,
    ) -> Result<(), BackendError> {
        self.execute_sql(ensure_sql).await?;
        let rows = self.execute_sql(exists_sql).await?;
        let already_reflected = rows.iter().any(|batch| batch.num_rows() > 0);
        if already_reflected {
            return Err(BackendError::already_reflected(
                "delta already reflected in the reconciliation ledger (best-effort existence check)",
            ));
        }
        self.execute_sql(insert_sql).await?;
        self.execute_sql(action_sql).await?;
        Ok(())
    }

    /// Run zero or more idempotent `ensure_sqls` DDL statements, then zero or
    /// more `pre_write_sqls` bookkeeping statements, then `write_group`, with
    /// the `pre_write_sqls` + `write_group` portion sharing one backend
    /// transaction where the backend can provide one — the generalised seam
    /// underneath every "record something alongside a write" caller
    /// (observed output deltas, T5; the re-run-tolerant keyed-model merge
    /// ledger, `docs/specs/incremental_shapes.md` §"The transactional
    /// frontier write (merge ledger)"). `ensure_sqls` run first and outside
    /// that transaction — same precedent as [`Backend::fold_ledger_delta`]'s
    /// `ensure_sql` handling — because idempotent `CREATE TABLE IF NOT
    /// EXISTS` DDL is safe standalone and keeping DDL out of the transaction
    /// avoids backend-specific DDL-vs-constraint-check interactions.
    /// `pre_write_sqls` run in order, before `write_group`, because a
    /// bookkeeping record commonly reads pre-write target state (the
    /// observed-delta record does; see
    /// [`Backend::execute_conditional_write_and_record_observed_delta`]).
    ///
    /// Default implementation is a best-effort, **non-atomic** fallback
    /// (each `ensure_sqls` entry, then each `pre_write_sqls` entry, then
    /// [`Backend::execute_statement_group`]) for any backend that does not
    /// override it — the same precedent as [`Backend::fold_ledger_delta`]'s
    /// default. A backend that can wrap the pre-write statements and the
    /// write in a native transaction (DuckDB) should override this so a
    /// failed write never leaves a stale bookkeeping record behind, and a
    /// failed pre-write record never lets the write proceed unrecorded.
    async fn execute_write_with_bookkeeping(
        &self,
        ensure_sqls: &[String],
        pre_write_sqls: &[String],
        write_group: &StatementGroup,
    ) -> Result<(), BackendError> {
        for ensure_sql in ensure_sqls {
            self.execute_sql(ensure_sql).await?;
        }
        for pre_write_sql in pre_write_sqls {
            self.execute_sql(pre_write_sql).await?;
        }
        self.execute_statement_group(write_group).await?;
        Ok(())
    }

    /// Record a conditional write's observed output delta, THEN execute the
    /// write itself, both in the same backend transaction (T5,
    /// `docs/specs/incremental_models.md` §"The graph layer" — "Observed
    /// deltas on model edges"): a delta visible without its write, or a
    /// write without its delta, breaks propagation soundness. `record_sql`
    /// must run **before** `write_group` — it reads the target table's
    /// pre-write state to compute the changed-row set (`target` vs.
    /// `source` in the same `IS DISTINCT FROM` shape the write's own
    /// suppression guard uses); running it after the write would compare
    /// the target against itself and record nothing. `ensure_sql` creates
    /// the observed-delta table if absent (idempotent DDL, safe
    /// standalone); `write_group` is the conditional write's own
    /// already-emitted [`StatementGroup`] (unchanged — this method does not
    /// alter what gets written); `record_sql` is the warehouse-resident
    /// upsert of the changed-key/partition set this write is about to touch
    /// (`smelt_state::ddl_duckdb::generate_observed_delta_upsert_sql`). All
    /// three strings/groups come from a caller with the `smelt-state`
    /// dependency — this trait does not depend on it, only executes the SQL
    /// text a caller built, same precedent as [`Backend::fold_ledger_delta`].
    ///
    /// Thin delegation to [`Backend::execute_write_with_bookkeeping`] — the
    /// one seam a backend needs to override to get a real transaction here;
    /// this method itself is never overridden directly.
    async fn execute_conditional_write_and_record_observed_delta(
        &self,
        ensure_sql: &str,
        write_group: &StatementGroup,
        record_sql: &str,
    ) -> Result<(), BackendError> {
        self.execute_write_with_bookkeeping(
            &[ensure_sql.to_string()],
            &[record_sql.to_string()],
            write_group,
        )
        .await
    }

    /// Refresh the row-content fingerprint sidecar (F3,
    /// `docs/plans/20260715-composed-axes-conditional-maintenance.md` Phase
    /// F3; `docs/specs/sources.md` §"The fingerprint sidecar" —
    /// "Transactionality") in the SAME backend transaction as `write_group`
    /// — the consuming write this refresh rides with: "a failed write
    /// leaves no digest update, so a re-run recomputes the same delta
    /// rather than silently treating a half-committed key as already
    /// seen." Call this AFTER the diff that read the changed-key set
    /// `write_group` is about to consume — refreshing first would make a
    /// subsequent diff compare the source against itself and observe no
    /// changes, the same before/after ordering constraint
    /// `execute_conditional_write_and_record_observed_delta`'s own
    /// `record_sql` documents (there, reversed, because that record reads
    /// PRE-write target state; here the source being digested is external
    /// and unaffected by `write_group`, so only the diff-then-refresh
    /// ordering matters, not this call's own internal ordering).
    ///
    /// `ensure_sql` creates the sidecar table if absent (idempotent DDL,
    /// safe standalone); `refresh_sql`/`gc_sql` come from
    /// `smelt_state::ddl_duckdb::generate_fingerprint_sidecar_refresh_sql`/
    /// `_gc_sql` — the upsert of every currently-observed key's digest and
    /// the GC delete of keys no longer present, both built over the SAME
    /// digest-select query the diff read (so a key that disappeared
    /// between the diff and this refresh cannot be silently left
    /// un-GC'd). All strings/groups come from a caller with the
    /// `smelt-state`/`smelt-logical` dependency — this trait does not
    /// depend on either, only executes the SQL text a caller built, same
    /// precedent as [`Backend::fold_ledger_delta`].
    ///
    /// Default implementation is a best-effort, **non-atomic** fallback
    /// (`ensure_sql`, then `write_group`, then `refresh_sql`, then
    /// `gc_sql`, as separate statements) for any backend that does not
    /// override it — the same precedent as
    /// [`Backend::execute_conditional_write_and_record_observed_delta`]'s
    /// default. A backend that can wrap the write and the sidecar refresh
    /// in a native transaction (DuckDB) should override this so a failed
    /// write never leaves a stale sidecar digest behind, and a failed
    /// refresh never lets the write proceed with an un-refreshed sidecar.
    async fn execute_write_and_refresh_fingerprint_sidecar(
        &self,
        ensure_sql: &str,
        write_group: &StatementGroup,
        refresh_sql: &str,
        gc_sql: &str,
    ) -> Result<(), BackendError> {
        self.execute_sql(ensure_sql).await?;
        self.execute_statement_group(write_group).await?;
        self.execute_sql(refresh_sql).await?;
        self.execute_sql(gc_sql).await?;
        Ok(())
    }
}
