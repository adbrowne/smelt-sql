//! Trino/Iceberg DDL generation from abstract `SchemaOperation`s.
//!
//! Every rule below is a *measured* Trino/Iceberg fact, established against
//! a live coordinator by `scripts/trino-probe-ddl.sh` — not a reading of
//! Trino's or Iceberg's documentation. `docs/specs/schema_evolution.md`
//! §"Trino/Iceberg DDL" is the narrative form of this table; keep the two in
//! sync.
//!
//! | `SchemaOperation` | Trino/Iceberg DDL | Notes |
//! |---|---|---|
//! | `AddColumn` (nullable, no default) | `ADD COLUMN c t` | |
//! | `AddColumn` (nullable, `default:`) | `ADD COLUMN c t`, then `UPDATE … WHERE c IS NULL` | Iceberg has no persistent `DEFAULT`; only existing rows are backfilled |
//! | `AddColumn` (NOT NULL) | refused | `This connector does not support adding not null columns` |
//! | `RemoveColumn` | `DROP COLUMN c` | |
//! | `WidenColumnType` | `ALTER COLUMN c SET DATA TYPE t` | works for scalars, arrays, and maps alike |
//! | `ChangeNullability` (relax) | `[UPDATE … WHERE c IS NULL,] ALTER COLUMN c DROP NOT NULL` | `DROP NOT NULL` is the one form whose column name is emitted **unquoted** — see `operations::ddl_for_operation` |
//! | `ChangeNullability` (tighten) | refused | no `SET NOT NULL` form exists at all |
//! | `AddStructField` (no default) | dotted `ADD COLUMN s.b t` | |
//! | `AddStructField` (`default:`) | refused | no dotted `UPDATE` target, no `DEFAULT` |
//! | `RemoveStructField` | dotted `DROP COLUMN s.b` | safe: Iceberg tracks columns by field ID |
//! | `WidenNestedType` (struct field / array element) | dotted `ALTER COLUMN s.a SET DATA TYPE t` | |
//! | `WidenNestedType` (map value) | `ALTER COLUMN m SET DATA TYPE MAP(VARCHAR, t)` | key type not carried by the operation — see `operations::ddl_for_operation` |
//! | `BackfillColumn` | `UPDATE … SET c = expr` | no `WHERE` requirement, unlike BigQuery |
//! | `RewriteColumn` | refused | no `USING` clause; no safe two-step form |
//!
//! What Trino/Iceberg cannot express resolves to a full refresh carrying a
//! reason that names the column and the limitation — the alternative is DDL
//! the coordinator rejects mid-run.

mod operations;
mod types;

pub use types::trino_type_sql;

use crate::schema_tracking::SchemaOperation;
use operations::ddl_for_operation;
use smelt_dialect::BackendCapabilities;
use types::qualified;

/// Result of planning a migration for Trino/Iceberg.
///
/// Trino has no counterpart to Spark's `TableRewrite` or `MergeSchemaWrite`
/// strategies — either the coordinator can express the change as DDL, or
/// the model is rebuilt from source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrinoMigration {
    /// DDL/DML statements to execute in order.
    Statements(Vec<String>),
    /// Trino/Iceberg cannot express this change — needs `--allow-full-refresh`.
    FullRefreshRequired { reason: String },
}

/// Generate Trino migration statements from a list of `SchemaOperation`s.
///
/// # Arguments
/// * `catalog` — the Iceberg catalog (e.g. `iceberg`)
/// * `schema` — the schema within that catalog
/// * `table` — table name
/// * `ops` — abstract schema operations to execute
/// * `caps` — the capability set to gate struct/array/column-mapping forms
///   against, ordinarily `BackendCapabilities::trino_iceberg()`
pub fn generate_trino_ddl(
    catalog: &str,
    schema: &str,
    table: &str,
    ops: &[SchemaOperation],
    caps: &BackendCapabilities,
) -> TrinoMigration {
    let qualified_table = qualified(catalog, schema, table);
    let mut stmts = Vec::new();

    for op in ops {
        match ddl_for_operation(&qualified_table, op, caps) {
            Ok(sqls) => stmts.extend(sqls),
            Err(reason) => return TrinoMigration::FullRefreshRequired { reason },
        }
    }

    TrinoMigration::Statements(stmts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use smelt_types::DataType;

    fn stmts(ops: &[SchemaOperation]) -> Vec<String> {
        match generate_trino_ddl(
            "iceberg",
            "ds",
            "t",
            ops,
            &BackendCapabilities::trino_iceberg(),
        ) {
            TrinoMigration::Statements(s) => s,
            other => panic!("expected statements, got {:?}", other),
        }
    }

    fn reason(ops: &[SchemaOperation]) -> String {
        match generate_trino_ddl(
            "iceberg",
            "ds",
            "t",
            ops,
            &BackendCapabilities::trino_iceberg(),
        ) {
            TrinoMigration::FullRefreshRequired { reason } => reason,
            other => panic!("expected a refusal, got {:?}", other),
        }
    }

    #[test]
    fn add_nullable_column_is_one_statement() {
        assert_eq!(
            stmts(&[SchemaOperation::AddColumn {
                name: "amount".into(),
                data_type: DataType::BigInt,
                nullable: true,
                default_expr: None,
            }]),
            vec!["ALTER TABLE \"iceberg\".\"ds\".\"t\" ADD COLUMN \"amount\" BIGINT"]
        );
    }

    #[test]
    fn add_column_with_default_backfills_existing_rows_only() {
        assert_eq!(
            stmts(&[SchemaOperation::AddColumn {
                name: "amount".into(),
                data_type: DataType::BigInt,
                nullable: true,
                default_expr: Some("0".into()),
            }]),
            vec![
                "ALTER TABLE \"iceberg\".\"ds\".\"t\" ADD COLUMN \"amount\" BIGINT",
                "UPDATE \"iceberg\".\"ds\".\"t\" SET \"amount\" = 0 WHERE \"amount\" IS NULL",
            ]
        );
    }

    #[test]
    fn not_null_column_add_is_refused_naming_the_column() {
        let why = reason(&[SchemaOperation::AddColumn {
            name: "amount".into(),
            data_type: DataType::BigInt,
            nullable: false,
            default_expr: Some("0".into()),
        }]);
        assert!(why.contains("amount") && why.contains("not null"), "{why}");
    }

    #[test]
    fn widen_uses_set_data_type() {
        assert_eq!(
            stmts(&[SchemaOperation::WidenColumnType {
                name: "amount".into(),
                from: DataType::Integer,
                to: DataType::BigInt,
            }]),
            vec![
                "ALTER TABLE \"iceberg\".\"ds\".\"t\" ALTER COLUMN \"amount\" SET DATA TYPE BIGINT"
            ]
        );
    }

    #[test]
    fn nullability_relaxes_but_never_tightens() {
        assert_eq!(
            stmts(&[SchemaOperation::ChangeNullability {
                name: "amount".into(),
                to_nullable: true,
                default_expr: None,
            }]),
            vec!["ALTER TABLE \"iceberg\".\"ds\".\"t\" ALTER COLUMN amount DROP NOT NULL"]
        );
        let why = reason(&[SchemaOperation::ChangeNullability {
            name: "amount".into(),
            to_nullable: false,
            default_expr: Some("0".into()),
        }]);
        assert!(why.contains("SET NOT NULL"), "{why}");
    }

    #[test]
    fn drop_column_is_ddl() {
        assert_eq!(
            stmts(&[SchemaOperation::RemoveColumn {
                name: "amount".into()
            }]),
            vec!["ALTER TABLE \"iceberg\".\"ds\".\"t\" DROP COLUMN \"amount\""]
        );
    }

    #[test]
    fn add_struct_field_without_default_is_dotted_ddl() {
        assert_eq!(
            stmts(&[SchemaOperation::AddStructField {
                column: "meta".into(),
                path: vec![],
                field_name: "b".into(),
                field_type: DataType::Integer,
                default_expr: None,
            }]),
            vec!["ALTER TABLE \"iceberg\".\"ds\".\"t\" ADD COLUMN \"meta\".\"b\" INTEGER"]
        );
    }

    #[test]
    fn add_struct_field_with_default_is_refused() {
        let why = reason(&[SchemaOperation::AddStructField {
            column: "meta".into(),
            path: vec![],
            field_name: "b".into(),
            field_type: DataType::Integer,
            default_expr: Some("0".into()),
        }]);
        assert!(why.contains("meta") && why.contains("b"), "{why}");
    }

    #[test]
    fn remove_struct_field_is_dotted_ddl_unlike_spark_and_bigquery() {
        assert_eq!(
            stmts(&[SchemaOperation::RemoveStructField {
                column: "meta".into(),
                path: vec![],
                field_name: "b".into(),
            }]),
            vec!["ALTER TABLE \"iceberg\".\"ds\".\"t\" DROP COLUMN \"meta\".\"b\""]
        );
    }

    #[test]
    fn widen_nested_struct_field_uses_dotted_set_data_type() {
        assert_eq!(
            stmts(&[SchemaOperation::WidenNestedType {
                column: "meta".into(),
                path: vec!["a".into()],
                from: DataType::Integer,
                to: DataType::BigInt,
            }]),
            vec!["ALTER TABLE \"iceberg\".\"ds\".\"t\" ALTER COLUMN \"meta\".\"a\" SET DATA TYPE BIGINT"]
        );
    }

    #[test]
    fn widen_map_value_rebuilds_the_whole_map_type() {
        assert_eq!(
            stmts(&[SchemaOperation::WidenNestedType {
                column: "m".into(),
                path: vec!["value".into()],
                from: DataType::Integer,
                to: DataType::BigInt,
            }]),
            vec!["ALTER TABLE \"iceberg\".\"ds\".\"t\" ALTER COLUMN \"m\" SET DATA TYPE MAP(VARCHAR, BIGINT)"]
        );
    }

    #[test]
    fn backfill_needs_no_where_clause() {
        assert_eq!(
            stmts(&[SchemaOperation::BackfillColumn {
                name: "amount".into(),
                expression: "0".into(),
            }]),
            vec!["UPDATE \"iceberg\".\"ds\".\"t\" SET \"amount\" = 0"]
        );
    }

    #[test]
    fn struct_field_ddl_is_gated_by_capabilities() {
        let mut caps = BackendCapabilities::trino_iceberg();
        caps.supports_struct_field_ddl = false;
        let op = SchemaOperation::AddStructField {
            column: "meta".into(),
            path: vec![],
            field_name: "b".into(),
            field_type: DataType::Integer,
            default_expr: None,
        };
        match generate_trino_ddl("iceberg", "ds", "t", &[op], &caps) {
            TrinoMigration::FullRefreshRequired { reason } => {
                assert!(reason.contains("meta") && reason.contains("b"), "{reason}");
            }
            other => panic!("expected a refusal, got {:?}", other),
        }
    }

    #[test]
    fn struct_field_drop_is_gated_by_column_mapping() {
        let mut caps = BackendCapabilities::trino_iceberg();
        caps.supports_column_mapping = false;
        let op = SchemaOperation::RemoveStructField {
            column: "meta".into(),
            path: vec![],
            field_name: "b".into(),
        };
        match generate_trino_ddl("iceberg", "ds", "t", &[op], &caps) {
            TrinoMigration::FullRefreshRequired { reason } => {
                assert!(reason.contains("meta") && reason.contains("b"), "{reason}");
            }
            other => panic!("expected a refusal, got {:?}", other),
        }
    }

    #[test]
    fn rewrite_column_is_always_refused() {
        let why = reason(&[SchemaOperation::RewriteColumn {
            column: "meta".into(),
            target_type: DataType::Integer,
            using_expr: "CAST(meta AS INTEGER)".into(),
        }]);
        assert!(why.contains("meta") && why.contains("USING"), "{why}");
    }
}
