//! SQL dialect definitions and backend capabilities.

/// SQL dialect used by a backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SqlDialect {
    /// DuckDB SQL dialect
    DuckDB,
    /// Apache Spark SQL dialect
    SparkSQL,
    /// PostgreSQL dialect
    PostgreSQL,
    /// Google BigQuery (GoogleSQL) dialect
    BigQuery,
}

impl SqlDialect {
    /// Get a human-readable name for this dialect.
    pub fn name(&self) -> &'static str {
        match self {
            SqlDialect::DuckDB => "DuckDB",
            SqlDialect::SparkSQL => "Spark SQL",
            SqlDialect::PostgreSQL => "PostgreSQL",
            SqlDialect::BigQuery => "BigQuery",
        }
    }
}

/// Capabilities of a backend.
///
/// Used to determine what SQL features can be used directly vs. need rewriting.
#[derive(Debug, Clone)]
pub struct BackendCapabilities {
    /// Supports QUALIFY clause for window function filtering
    pub supports_qualify: bool,

    /// Supports CREATE OR REPLACE TABLE syntax
    pub supports_create_or_replace_table: bool,

    /// Supports CREATE OR REPLACE VIEW syntax
    pub supports_create_or_replace_view: bool,

    /// Supports MERGE statement (upsert)
    pub supports_merge: bool,

    /// Supports PIVOT/UNPIVOT natively
    pub supports_pivot: bool,

    /// Supports DATE 'YYYY-MM-DD' literal syntax
    pub supports_date_literal: bool,

    /// Supports || for string concatenation
    pub supports_concat_operator: bool,

    /// Supports arrays with [a, b, c] syntax
    pub supports_array_literal: bool,

    /// Supports transactional DDL (can rollback CREATE TABLE)
    pub supports_transactional_ddl: bool,

    /// Supports :: cast operator (PostgreSQL-style)
    pub supports_double_colon_cast: bool,

    /// Supports trailing commas in SELECT and GROUP BY lists
    pub supports_trailing_commas: bool,

    /// Supports native INSERT OVERWRITE for partition replacement
    pub supports_insert_overwrite: bool,

    /// Supports native incremental-view maintenance (IVM): the backend can
    /// maintain a declared query as a continuously-refreshed materialized
    /// object with no smelt-driven refresh loop (e.g. Databricks Enzyme,
    /// Snowflake Dynamic Tables). Gates `refresh: materialized_view`
    /// (`materialized_view.md` §"No silent fallback"); `true` on BigQuery
    /// alone, `false` on every other backend today.
    pub supports_native_ivm: bool,

    /// Supports retraction (inverting / reprocessing a prior input) within
    /// its native IVM runtime. Meaningful only alongside `supports_native_ivm`;
    /// does not describe smelt-driven `cumulative` retraction, which is a
    /// per-model property of the aggregator algebra, not a backend flag
    /// (`multi_backend.md` §"Incremental-view-maintenance capabilities").
    pub supports_retraction: bool,

    // --- Schema evolution capabilities ---
    /// Supports `ALTER TABLE ADD COLUMN s.field` (struct field DDL via dot-notation)
    pub supports_struct_field_ddl: bool,

    /// Supports `ALTER COLUMN TYPE ... USING expr` (rewrite column in-place)
    pub supports_alter_column_using: bool,

    /// Supports `ALTER TABLE ADD COLUMN items.element.field` (nested array struct DDL)
    pub supports_nested_array_ddl: bool,

    /// Supports mergeSchema on write (Spark only)
    pub supports_merge_schema_write: bool,

    /// Supports ID-based column mapping (Delta only)
    pub supports_column_mapping: bool,

    /// Supports pipe SQL (`|>`) syntax natively.
    ///
    /// When `false`, the dialect printer rewrites pipe queries to standard SQL
    /// before emitting. BigQuery is the only backend reporting `true`, on the
    /// strength of GoogleSQL's native pipe support.
    pub supports_pipe_syntax: bool,

    /// Backend requires explicit schema creation during session init.
    ///
    /// When `true`, the backend creates the target schema (via `ensure_schema`)
    /// before selecting it as the current database. All current backends require
    /// this — it is `true` universally and exists so the capability matrix stays
    /// accurate and callers can assert the contract.
    pub requires_schema_init: bool,

    /// Supports a column-scoped `MERGE` — the physical primitive behind
    /// `smelt_logical::maintenance::Technique::ColumnScopedMerge`
    /// (`docs/specs/model_transforms.md` §"Dimension-driven horizon-bounded
    /// MERGE") and the open write-pattern registry's `column` /
    /// `keyed_conditional` entries (`docs/specs/incremental_models.md`
    /// §"Per-cell write addressing"). This is the capability struct's copy
    /// of what was formerly `Backend::supports_column_scoped_merge()` — a
    /// genuine backend-capability gate, not a policy choice: a backend that
    /// cannot run a targeted `MERGE` against a full-row source projection
    /// must drop the technique from admission at plan time. `false` unless
    /// the backend can execute `merge_into` against a source projection
    /// carrying the full target row.
    pub supports_column_scoped_merge: bool,
}

impl BackendCapabilities {
    /// Capabilities for DuckDB
    pub fn duckdb() -> Self {
        Self {
            supports_qualify: true,
            supports_create_or_replace_table: true,
            supports_create_or_replace_view: true,
            supports_merge: true,
            supports_pivot: true,
            supports_date_literal: true,
            supports_concat_operator: true,
            supports_array_literal: true,
            supports_transactional_ddl: true,
            supports_double_colon_cast: true,
            supports_trailing_commas: true,
            supports_insert_overwrite: false,
            supports_native_ivm: false,
            supports_retraction: false,
            // Schema evolution: DuckDB supports all struct/array DDL
            supports_struct_field_ddl: true,
            supports_alter_column_using: true,
            supports_nested_array_ddl: true,
            supports_merge_schema_write: false,
            supports_column_mapping: false,
            supports_pipe_syntax: false,
            requires_schema_init: true,
            supports_column_scoped_merge: true,
        }
    }

    /// Capabilities for Spark SQL with Delta table format.
    pub fn spark() -> Self {
        Self::spark_delta()
    }

    /// Capabilities for Spark SQL with Delta table format.
    pub fn spark_delta() -> Self {
        Self {
            supports_qualify: false,
            supports_create_or_replace_table: false,
            supports_create_or_replace_view: true,
            supports_merge: true,
            supports_pivot: true,
            supports_date_literal: false,
            supports_concat_operator: true,
            supports_array_literal: false,
            supports_transactional_ddl: false,
            supports_double_colon_cast: false,
            supports_trailing_commas: false,
            supports_insert_overwrite: true,
            // OSS Spark SQL has no native incremental-view-maintenance runtime.
            supports_native_ivm: false,
            supports_retraction: false,
            // Schema evolution: Delta supports struct field DDL and column mapping
            supports_struct_field_ddl: true,
            supports_alter_column_using: false,
            supports_nested_array_ddl: true,
            supports_merge_schema_write: true,
            supports_column_mapping: true,
            supports_pipe_syntax: false,
            requires_schema_init: true,
            // Delta MERGE supports `WHEN MATCHED THEN UPDATE SET *` over a
            // full-row source projection — the same shape DuckDB's MERGE
            // uses (`docs/specs/multi_backend.md` capability matrix).
            supports_column_scoped_merge: true,
        }
    }

    /// Capabilities for Spark SQL with Parquet table format (no Delta).
    pub fn spark_parquet() -> Self {
        Self {
            supports_qualify: false,
            supports_create_or_replace_table: false,
            supports_create_or_replace_view: true,
            supports_merge: false, // No MERGE without Delta
            supports_pivot: true,
            supports_date_literal: false,
            supports_concat_operator: true,
            supports_array_literal: false,
            supports_transactional_ddl: false,
            supports_double_colon_cast: false,
            supports_trailing_commas: false,
            supports_insert_overwrite: true,
            // OSS Spark SQL has no native incremental-view-maintenance runtime.
            supports_native_ivm: false,
            supports_retraction: false,
            // Empirically verified (W6·P2, Spark 4.1.x): ALTER TABLE … ADD COLUMNS
            // with a qualified struct path is rejected — [UNSUPPORTED_FEATURE.TABLE_OPERATION].
            supports_struct_field_ddl: false,
            supports_alter_column_using: false,
            supports_nested_array_ddl: false,
            supports_merge_schema_write: true,
            supports_column_mapping: false,
            supports_pipe_syntax: false,
            requires_schema_init: true,
            supports_column_scoped_merge: false,
        }
    }

    /// Capabilities for Google BigQuery (GoogleSQL).
    ///
    /// Every flag below was established by executing the statement it names
    /// against a live warehouse (`scripts/bigquery-probe.sh`), not read from
    /// documentation — the capability matrix in `docs/specs/multi_backend.md`
    /// §Surface is normative and the conformance test asserts this constructor
    /// against it.
    pub fn bigquery() -> Self {
        Self {
            supports_qualify: true,
            supports_create_or_replace_table: true,
            supports_create_or_replace_view: true,
            supports_merge: true,
            supports_pivot: true,
            supports_date_literal: true,
            supports_concat_operator: true,
            supports_array_literal: true,
            supports_transactional_ddl: true,
            // GoogleSQL has no `::` cast operator: `SELECT 1::INT64` is a syntax error.
            supports_double_colon_cast: false,
            supports_trailing_commas: true,
            // No `INSERT OVERWRITE` in GoogleSQL; partition replacement lowers to a
            // scoped DELETE + INSERT.
            supports_insert_overwrite: false,
            // The one backend that advertises native IVM. smelt emits
            // `CREATE OR REPLACE MATERIALIZED VIEW` and then owns nothing of the
            // refresh loop: BigQuery keeps the object current and serves it by
            // combining the materialized data with a live delta over the base table,
            // so a read straight after a write already reflects it. smelt runs no
            // combiner and keeps no ledger for these models
            // (`materialized_view.md` §Constraints item 4). Eligibility is the
            // engine's verdict alone — an unsupported query shape is relayed
            // verbatim, never pre-empted by a smelt-side check.
            supports_native_ivm: true,
            // BigQuery's IVM does not invert a prior contribution; retraction-shaped
            // queries are refused at creation rather than maintained.
            supports_retraction: false,
            supports_struct_field_ddl: true,
            // GoogleSQL has no `USING` clause on ALTER COLUMN at all (syntax error);
            // the bare `SET DATA TYPE` form permits only assignable widenings, so a
            // conversion needing an expression is a table rewrite.
            supports_alter_column_using: false,
            supports_nested_array_ddl: true,
            // A write naming a column the table lacks is rejected, not auto-added.
            supports_merge_schema_write: false,
            supports_column_mapping: true,
            // The first backend to advertise native pipe syntax.
            supports_pipe_syntax: true,
            // A write into a dataset that does not exist is refused with
            // `Not found: Dataset ...`, so the dataset must be created first.
            requires_schema_init: true,
            supports_column_scoped_merge: true,
        }
    }

    /// Capabilities for PostgreSQL
    pub fn postgresql() -> Self {
        Self {
            supports_qualify: false,
            supports_create_or_replace_table: false,
            supports_create_or_replace_view: true,
            supports_merge: true,
            supports_pivot: false,
            supports_date_literal: true,
            supports_concat_operator: true,
            supports_array_literal: false,
            supports_transactional_ddl: true,
            supports_double_colon_cast: true,
            supports_trailing_commas: false,
            supports_insert_overwrite: false,
            // No backend advertises native IVM today (`multi_backend.md` §IVM).
            supports_native_ivm: false,
            supports_retraction: false,
            // Schema evolution: PostgreSQL has limited struct support
            supports_struct_field_ddl: false,
            supports_alter_column_using: true,
            supports_nested_array_ddl: false,
            supports_merge_schema_write: false,
            supports_column_mapping: false,
            supports_pipe_syntax: false,
            requires_schema_init: true,
            supports_column_scoped_merge: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_duckdb_schema_evolution_capabilities() {
        let caps = BackendCapabilities::duckdb();
        assert!(caps.supports_struct_field_ddl);
        assert!(caps.supports_alter_column_using);
        assert!(caps.supports_nested_array_ddl);
        assert!(!caps.supports_merge_schema_write);
        assert!(!caps.supports_column_mapping);
    }

    #[test]
    fn test_spark_delta_schema_evolution_capabilities() {
        let caps = BackendCapabilities::spark_delta();
        assert!(caps.supports_struct_field_ddl);
        assert!(!caps.supports_alter_column_using);
        assert!(caps.supports_nested_array_ddl); // empirically verified true in W7·P2
        assert!(caps.supports_merge_schema_write);
        assert!(caps.supports_column_mapping);
    }

    #[test]
    fn test_spark_parquet_schema_evolution_capabilities() {
        let caps = BackendCapabilities::spark_parquet();
        assert!(!caps.supports_struct_field_ddl); // false: empirically verified in W6·P2
        assert!(!caps.supports_alter_column_using);
        assert!(!caps.supports_nested_array_ddl);
        assert!(caps.supports_merge_schema_write);
        assert!(!caps.supports_column_mapping);
        // Parquet doesn't support MERGE (no Delta)
        assert!(!caps.supports_merge);
    }

    #[test]
    fn test_spark_default_is_delta() {
        let spark_default = BackendCapabilities::spark();
        let spark_delta = BackendCapabilities::spark_delta();
        assert_eq!(
            spark_default.supports_column_mapping,
            spark_delta.supports_column_mapping
        );
        assert_eq!(
            spark_default.supports_merge_schema_write,
            spark_delta.supports_merge_schema_write
        );
        assert_eq!(spark_default.supports_merge, spark_delta.supports_merge);
    }

    #[test]
    fn test_postgresql_schema_evolution_capabilities() {
        let caps = BackendCapabilities::postgresql();
        assert!(!caps.supports_struct_field_ddl);
        assert!(caps.supports_alter_column_using);
        assert!(!caps.supports_nested_array_ddl);
        assert!(!caps.supports_merge_schema_write);
        assert!(!caps.supports_column_mapping);
    }
}
