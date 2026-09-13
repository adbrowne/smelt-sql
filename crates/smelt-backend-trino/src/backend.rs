//! The `Backend` trait implementation over the live Trino/Iceberg tier.
//!
//! [`Backend::capabilities`] is answered provisionally, per the outcome's
//! decision log (`docs/outcomes/20260913-trino-target-spine/outcome.md`,
//! 2026-09-13 "the spec's Trino capability column enters as `?`, not as a
//! documentation-read guess"): it returns an all-`false` profile until phase
//! 8 measures the real one by execution. `create_materialized_view_as`
//! inherits the trait's erroring default — Trino has no native IVM to
//! override it with.
//!
//! `delete_partitions`, `insert_into_from_query` and `insert_overwrite` also
//! refuse by name: this outcome's Out of scope section reserves "the
//! incremental/maintenance families" for
//! `docs/outcomes/20260913-trino-incremental`, and DuckDB's own DELETE+INSERT
//! emulation for these methods bakes in transactionality and partition-axis
//! assumptions this outcome has not measured against Iceberg — a guess here
//! would need re-deciding there anyway, so refusing names the gap rather
//! than papering over it with an implementation nothing has verified.

use arrow::array::{Array, Int64Array, RecordBatch};
use arrow::datatypes::SchemaRef;
use async_trait::async_trait;
use smelt_backend::{Backend, BackendCapabilities, BackendError, PartitionRange, SqlDialect};
use smelt_dialect::NullSafeEqualitySpelling;

use crate::arrow_convert::{arrow_type_to_trino_type, render_trino_literal};
use crate::client::TrinoClient;
use crate::config::TrinoClientConfig;

/// The row bound per `INSERT` statement `load_table` issues, measured in
/// phase 7 (`docs/outcomes/20260913-trino-target-spine/phases/07-summary.md`)
/// against the live tier: it keeps each statement's HTTP body and Trino's own
/// parse time bounded regardless of how large the seed batch is.
const INSERT_CHUNK_SIZE: usize = 1000;

/// A `Backend` over Trino's `/v1/statement` HTTP protocol and an Iceberg
/// REST catalog. Holds one [`TrinoClient`] shared across every call.
pub struct TrinoBackend {
    client: TrinoClient,
    catalog: String,
}

impl TrinoBackend {
    /// Build a backend from a resolved client config. Makes no network
    /// call — the coordinator is only reached on the first actual query.
    pub fn new(config: TrinoClientConfig) -> Self {
        let catalog = config.catalog.clone();
        Self {
            client: TrinoClient::new(config),
            catalog,
        }
    }

    fn qualified_name(&self, schema: &str, name: &str) -> String {
        qualified_name(&self.catalog, schema, name)
    }
}

/// Build a catalog-qualified, double-quoted identifier: `"cat"."sch"."tbl"`.
/// Standard SQL identifier quoting, which Trino follows; an embedded `"` is
/// escaped by doubling it.
pub(crate) fn qualified_name(catalog: &str, schema: &str, name: &str) -> String {
    format!(
        "{}.{}.{}",
        quote_identifier(catalog),
        quote_identifier(schema),
        quote_identifier(name)
    )
}

fn quote_identifier(ident: &str) -> String {
    format!("\"{}\"", ident.replace('"', "\"\""))
}

/// Escape a single-quoted SQL string literal for an `information_schema`
/// predicate — never used for an identifier, only a filter value.
fn escape_string_literal(s: &str) -> String {
    s.replace('\'', "''")
}

/// The statements `load_table` executes once nullability has passed and
/// every literal is rendered: one `CREATE TABLE` and a bounded sequence of
/// `INSERT` statements. Kept as data so the plan is testable without a
/// server — `TrinoBackend::load_table` is the only caller that turns it into
/// HTTP requests.
#[derive(Debug)]
struct LoadPlan {
    create_table_sql: String,
    insert_sqls: Vec<String>,
}

/// Build the whole `load_table` plan for `qualified_table` from `arrow_schema`
/// and `batches` — nullability validation, the seed-type-set DDL mapping, and
/// chunked `INSERT INTO … SELECT CAST(…) FROM (VALUES …)` statements.
///
/// Nullability is checked **before any SQL is built**: a violation returns
/// immediately, so a caller never sees a partially-built plan.
fn build_load_plan(
    qualified_table: &str,
    arrow_schema: &SchemaRef,
    batches: &[RecordBatch],
    schema: &str,
    name: &str,
) -> Result<LoadPlan, BackendError> {
    for batch in batches {
        for (col_idx, field) in arrow_schema.fields().iter().enumerate() {
            if !field.is_nullable() {
                let array = batch.column(col_idx);
                if array.null_count() > 0 {
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

    let ddl_types = arrow_schema
        .fields()
        .iter()
        .map(|f| arrow_type_to_trino_type(f.data_type()))
        .collect::<Result<Vec<_>, _>>()?;

    let column_defs: Vec<String> = arrow_schema
        .fields()
        .iter()
        .zip(&ddl_types)
        .map(|(field, ddl_type)| {
            let nullability = if field.is_nullable() { "" } else { " NOT NULL" };
            format!("{} {ddl_type}{nullability}", field.name())
        })
        .collect();
    let create_table_sql = format!(
        "CREATE TABLE {qualified_table} ({})",
        column_defs.join(", ")
    );

    let column_names: Vec<&str> = arrow_schema
        .fields()
        .iter()
        .map(|f| f.name().as_str())
        .collect();
    let cols_list = column_names.join(", ");
    let value_col_names: Vec<String> = (0..column_names.len()).map(|i| format!("c{i}")).collect();
    let value_col_list = value_col_names.join(", ");
    let cast_list: Vec<String> = ddl_types
        .iter()
        .enumerate()
        .map(|(i, ddl_type)| format!("CAST(v.c{i} AS {ddl_type})"))
        .collect();
    let cast_list = cast_list.join(", ");

    let data_types: Vec<_> = arrow_schema
        .fields()
        .iter()
        .map(|f| f.data_type().clone())
        .collect();

    let mut insert_sqls = Vec::new();
    for batch in batches {
        let mut row_idx = 0;
        while row_idx < batch.num_rows() {
            let chunk_end = (row_idx + INSERT_CHUNK_SIZE).min(batch.num_rows());
            let mut value_rows = Vec::with_capacity(chunk_end - row_idx);
            for row in row_idx..chunk_end {
                let mut cells = Vec::with_capacity(batch.num_columns());
                for (col_idx, data_type) in data_types.iter().enumerate() {
                    cells.push(render_trino_literal(batch.column(col_idx), row, data_type)?);
                }
                value_rows.push(format!("({})", cells.join(", ")));
            }
            insert_sqls.push(format!(
                "INSERT INTO {qualified_table} ({cols_list}) \
                 SELECT {cast_list} FROM (VALUES {}) AS v({value_col_list})",
                value_rows.join(", ")
            ));
            row_idx = chunk_end;
        }
    }

    Ok(LoadPlan {
        create_table_sql,
        insert_sqls,
    })
}

/// Decode a single-row, single-column `bigint` result (a `count(*)`) into a
/// `usize`. Shared by `get_row_count` and `table_exists`.
fn decode_bigint_count(batches: &[RecordBatch], context: &str) -> Result<i64, BackendError> {
    let batch = batches.first().ok_or_else(|| {
        BackendError::execution_failed("trino", format!("no result row {context}"))
    })?;
    let counts = batch
        .column(0)
        .as_any()
        .downcast_ref::<Int64Array>()
        .ok_or_else(|| {
            BackendError::execution_failed(
                "trino",
                format!("expected a bigint count column {context}"),
            )
        })?;
    Ok(counts.value(0))
}

#[async_trait]
impl Backend for TrinoBackend {
    async fn execute_sql(&self, sql: &str) -> Result<Vec<RecordBatch>, BackendError> {
        self.client.execute(sql).await
    }

    async fn create_table_as(
        &self,
        schema: &str,
        name: &str,
        sql: &str,
    ) -> Result<(), BackendError> {
        let table = self.qualified_name(schema, name);
        // `supports_create_or_replace_table` is unmeasured (phase 8), so
        // this emulates replacement with DROP-then-CREATE rather than
        // assuming Trino/Iceberg accepts `CREATE OR REPLACE TABLE`.
        self.client
            .execute(&format!("DROP TABLE IF EXISTS {table}"))
            .await?;
        self.client
            .execute(&format!("CREATE TABLE {table} AS {sql}"))
            .await?;
        Ok(())
    }

    async fn create_view_as(
        &self,
        schema: &str,
        name: &str,
        sql: &str,
    ) -> Result<(), BackendError> {
        let view = self.qualified_name(schema, name);
        // Same reasoning as `create_table_as`: `supports_create_or_replace_view`
        // is unmeasured, so this drops first rather than assuming
        // `CREATE OR REPLACE VIEW`.
        self.client
            .execute(&format!("DROP VIEW IF EXISTS {view}"))
            .await?;
        self.client
            .execute(&format!("CREATE VIEW {view} AS {sql}"))
            .await?;
        Ok(())
    }

    async fn drop_table_if_exists(&self, schema: &str, name: &str) -> Result<(), BackendError> {
        let table = self.qualified_name(schema, name);
        self.client
            .execute(&format!("DROP TABLE IF EXISTS {table}"))
            .await?;
        Ok(())
    }

    async fn drop_view_if_exists(&self, schema: &str, name: &str) -> Result<(), BackendError> {
        let view = self.qualified_name(schema, name);
        self.client
            .execute(&format!("DROP VIEW IF EXISTS {view}"))
            .await?;
        Ok(())
    }

    async fn get_row_count(&self, schema: &str, name: &str) -> Result<usize, BackendError> {
        let table = self.qualified_name(schema, name);
        let batches = self
            .client
            .execute(&format!("SELECT count(*) FROM {table}"))
            .await?;
        let count = decode_bigint_count(&batches, &format!("counting {table}"))?;
        Ok(count as usize)
    }

    async fn get_preview(
        &self,
        schema: &str,
        name: &str,
        limit: usize,
    ) -> Result<Vec<RecordBatch>, BackendError> {
        let table = self.qualified_name(schema, name);
        self.client
            .execute(&format!("SELECT * FROM {table} LIMIT {limit}"))
            .await
    }

    async fn table_exists(&self, schema: &str, name: &str) -> Result<bool, BackendError> {
        // Catalog-scoped: `information_schema` is per-catalog in Trino, so
        // this queries `<catalog>.information_schema.tables` rather than
        // the unqualified form, which would resolve against the session's
        // default catalog instead of this backend's own.
        let sql = format!(
            "SELECT count(*) FROM {}.information_schema.tables \
             WHERE table_schema = '{}' AND table_name = '{}'",
            quote_identifier(&self.catalog),
            escape_string_literal(schema),
            escape_string_literal(name)
        );
        let batches = self.client.execute(&sql).await?;
        let count = decode_bigint_count(&batches, "checking table existence")?;
        Ok(count > 0)
    }

    async fn ensure_schema(&self, schema: &str) -> Result<(), BackendError> {
        let qualified = format!(
            "{}.{}",
            quote_identifier(&self.catalog),
            quote_identifier(schema)
        );
        self.client
            .execute(&format!("CREATE SCHEMA IF NOT EXISTS {qualified}"))
            .await?;
        Ok(())
    }

    fn dialect(&self) -> SqlDialect {
        SqlDialect::Trino
    }

    fn capabilities(&self) -> BackendCapabilities {
        // Provisional: every flag `false` until phase 8 measures the real
        // profile by execution against the live coordinator and replaces
        // this with `BackendCapabilities::trino_iceberg()`. Pinned by
        // `capabilities_are_provisionally_all_false`.
        BackendCapabilities {
            supports_qualify: false,
            supports_create_or_replace_table: false,
            supports_create_or_replace_view: false,
            supports_merge: false,
            supports_pivot: false,
            supports_date_literal: false,
            supports_concat_operator: false,
            supports_array_literal: false,
            supports_transactional_ddl: false,
            supports_double_colon_cast: false,
            supports_trailing_commas: false,
            supports_insert_overwrite: false,
            supports_native_ivm: false,
            supports_retraction: false,
            supports_struct_field_ddl: false,
            supports_alter_column_using: false,
            supports_nested_array_ddl: false,
            supports_merge_schema_write: false,
            supports_column_mapping: false,
            supports_pipe_syntax: false,
            requires_schema_init: false,
            supports_column_scoped_merge: false,
            dialect: SqlDialect::Trino,
            supports_pipe_set_drop_rename: false,
            // Unmeasured — Trino's actual spelling is phase 8's job. Picked
            // as the more common of the two spellings so a caller reading
            // this field before phase 8 lands sees a real enum variant
            // rather than an arbitrary default.
            null_safe_equality: NullSafeEqualitySpelling::IsNotDistinctFrom,
            supports_fingerprint_sidecar: false,
        }
    }

    async fn load_table(
        &self,
        schema: &str,
        name: &str,
        arrow_schema: SchemaRef,
        batches: Vec<RecordBatch>,
    ) -> Result<(), BackendError> {
        let table = self.qualified_name(schema, name);
        let plan = build_load_plan(&table, &arrow_schema, &batches, schema, name)?;

        self.client
            .execute(&format!("DROP TABLE IF EXISTS {table}"))
            .await?;
        self.client
            .execute(&format!("DROP VIEW IF EXISTS {table}"))
            .await?;
        self.client.execute(&plan.create_table_sql).await?;
        for insert_sql in &plan.insert_sqls {
            self.client.execute(insert_sql).await?;
        }

        Ok(())
    }

    async fn delete_partitions(
        &self,
        schema: &str,
        name: &str,
        _partition: &PartitionRange,
    ) -> Result<(), BackendError> {
        Err(BackendError::unsupported(
            self.dialect().name(),
            format!(
                "delete_partitions for '{schema}.{name}' — Trino's incremental/maintenance \
                 family lands in docs/outcomes/20260913-trino-incremental"
            ),
        ))
    }

    async fn insert_into_from_query(
        &self,
        schema: &str,
        name: &str,
        _sql: &str,
    ) -> Result<(), BackendError> {
        Err(BackendError::unsupported(
            self.dialect().name(),
            format!(
                "insert_into_from_query for '{schema}.{name}' — Trino's incremental/maintenance \
                 family lands in docs/outcomes/20260913-trino-incremental"
            ),
        ))
    }

    async fn insert_overwrite(
        &self,
        schema: &str,
        name: &str,
        _sql: &str,
        _partition: &PartitionRange,
    ) -> Result<(), BackendError> {
        Err(BackendError::unsupported(
            self.dialect().name(),
            format!(
                "insert_overwrite for '{schema}.{name}' — Trino's incremental/maintenance \
                 family lands in docs/outcomes/20260913-trino-incremental"
            ),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qualified_name_uses_catalog_schema_table() {
        assert_eq!(
            qualified_name("cat", "sch", "tbl"),
            "\"cat\".\"sch\".\"tbl\""
        );
    }

    #[test]
    fn qualified_name_escapes_an_embedded_quote() {
        assert_eq!(
            qualified_name("cat", "sch", "weird\"table"),
            "\"cat\".\"sch\".\"weird\"\"table\""
        );
    }

    fn backend() -> TrinoBackend {
        TrinoBackend::new(TrinoClientConfig {
            base_url: "http://localhost:18080".to_string(),
            user: "smelt".to_string(),
            catalog: "iceberg".to_string(),
            schema: "smelt_dev".to_string(),
            password: None,
        })
    }

    #[test]
    fn dialect_is_trino() {
        assert_eq!(backend().dialect(), SqlDialect::Trino);
    }

    #[test]
    fn capabilities_are_provisionally_all_false() {
        let caps = backend().capabilities();
        assert!(!caps.supports_qualify);
        assert!(!caps.supports_create_or_replace_table);
        assert!(!caps.supports_create_or_replace_view);
        assert!(!caps.supports_merge);
        assert!(!caps.supports_pivot);
        assert!(!caps.supports_date_literal);
        assert!(!caps.supports_concat_operator);
        assert!(!caps.supports_array_literal);
        assert!(!caps.supports_transactional_ddl);
        assert!(!caps.supports_double_colon_cast);
        assert!(!caps.supports_trailing_commas);
        assert!(!caps.supports_insert_overwrite);
        assert!(!caps.supports_native_ivm);
        assert!(!caps.supports_retraction);
        assert!(!caps.supports_struct_field_ddl);
        assert!(!caps.supports_alter_column_using);
        assert!(!caps.supports_nested_array_ddl);
        assert!(!caps.supports_merge_schema_write);
        assert!(!caps.supports_column_mapping);
        assert!(!caps.supports_pipe_syntax);
        assert!(!caps.requires_schema_init);
        assert!(!caps.supports_column_scoped_merge);
        assert!(!caps.supports_pipe_set_drop_rename);
        assert!(!caps.supports_fingerprint_sidecar);
    }

    fn int_schema(nullable: bool) -> SchemaRef {
        std::sync::Arc::new(arrow::datatypes::Schema::new(vec![
            arrow::datatypes::Field::new("id", arrow::datatypes::DataType::Int32, nullable),
        ]))
    }

    #[test]
    fn rejects_null_in_a_non_nullable_column_before_any_statement() {
        // The batch's own schema stays nullable (the way seed loading builds
        // record batches); `arrow_schema` is the authoritative, possibly
        // stricter, declaration `load_table` validates against.
        let batch_schema = int_schema(true);
        let arrow_schema = int_schema(false);
        let batch = RecordBatch::try_new(
            batch_schema,
            vec![std::sync::Arc::new(arrow::array::Int32Array::from(vec![
                Some(1),
                None,
            ]))],
        )
        .unwrap();

        let err = build_load_plan(
            "\"cat\".\"sch\".\"tbl\"",
            &arrow_schema,
            &[batch],
            "sch",
            "tbl",
        )
        .expect_err("a NULL in a non-nullable column must be refused");
        match err {
            BackendError::NullInNonNullableColumn {
                schema, table, row, ..
            } => {
                assert_eq!(schema, "sch");
                assert_eq!(table, "tbl");
                assert_eq!(row, 1);
            }
            other => panic!("expected NullInNonNullableColumn, got {other:?}"),
        }
    }

    #[test]
    fn chunks_rows_into_bounded_insert_statements() {
        let schema = int_schema(true);
        let row_count = INSERT_CHUNK_SIZE * 2 + 5;
        let values: Vec<Option<i32>> = (0..row_count as i32).map(Some).collect();
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![std::sync::Arc::new(arrow::array::Int32Array::from(values))],
        )
        .unwrap();

        let plan = build_load_plan("\"cat\".\"sch\".\"tbl\"", &schema, &[batch], "sch", "tbl")
            .expect("plan must build");
        assert_eq!(plan.insert_sqls.len(), 3);
        // Every statement's row count is at the bound — count value tuples in
        // each statement's `VALUES (...), (...), ...` clause.
        for (i, sql) in plan.insert_sqls.iter().enumerate() {
            let values_clause = sql.split("VALUES ").nth(1).expect("VALUES clause present");
            let values_clause = values_clause
                .split(") AS v(")
                .next()
                .expect("closing AS v(");
            let tuple_count = values_clause.matches("), (").count() + 1;
            let expected = if i < 2 { INSERT_CHUNK_SIZE } else { 5 };
            assert_eq!(tuple_count, expected, "statement {i} row count");
        }
    }

    #[tokio::test]
    async fn create_materialized_view_as_inherits_the_erroring_default() {
        let err = backend()
            .create_materialized_view_as("sch", "tbl", "SELECT 1")
            .await
            .expect_err("must inherit the trait's erroring default");
        assert!(matches!(err, BackendError::UnsupportedFeature { .. }));
    }
}
