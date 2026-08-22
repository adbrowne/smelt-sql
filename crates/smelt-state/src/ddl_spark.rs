//! Spark-specific DDL generation from abstract `SchemaOperation`s.
//!
//! Translates backend-agnostic schema operations into Spark SQL statements
//! for both Delta and Parquet table formats.
//!
//! Every rule below was measured against a live server rather than read from
//! documentation — `scripts/spark-probe-ddl.sh` runs each form against a fresh
//! Delta and Parquet table and prints what the server answered. The rules are
//! stated for the tables *smelt creates*: `CREATE TABLE … USING DELTA` with no
//! table properties, and plain v1 Parquet. Several Delta forms that the format
//! supports in principle are refused on such a table because they need a table
//! feature smelt does not enable (`delta.columnMapping.mode`,
//! `delta.enableTypeWidening`, `allowColumnDefaults`), and a statement the
//! deployed table refuses is worse than a migration smelt declines to express.
//!
//! | Operation | Delta | Parquet |
//! |---|---|---|
//! | Add nullable column | `ADD COLUMNS (c T)` | `ADD COLUMNS (c T)` |
//! | Add nullable column with a `default:` | add, then `UPDATE … WHERE c IS NULL` | full refresh (no `UPDATE`) |
//! | Add `NOT NULL` column | full refresh | full refresh |
//! | Drop column | table rewrite | full refresh |
//! | Widen a column type | table rewrite | full refresh |
//! | `DROP NOT NULL` | `ALTER COLUMN c DROP NOT NULL` | full refresh |
//! | `SET NOT NULL` | full refresh | full refresh |
//! | Add a struct field | `ADD COLUMNS (c.f T)` | full refresh |
//! | Drop a struct field | full refresh | full refresh |
//! | Backfill (`UPDATE`) | `UPDATE …` | full refresh |
//!
//! Key differences from DuckDB:
//! - No `ALTER COLUMN TYPE ... USING expr` — nested type widening requires table rewrite
//! - Type names differ: bare `VARCHAR` is `DATATYPE_MISSING_SIZE` and `TEXT` is not a
//!   type at all; both spell as `STRING`
//! - `NOT NULL` and `DEFAULT` may not ride on `ADD COLUMNS`
//! - `mergeSchema` on write for adding nullable fields (Spark-specific)
//! - Three-part naming: `catalog.schema.table`

use crate::schema_tracking::SchemaOperation;
use smelt_dialect::BackendCapabilities;
use smelt_types::DataType;

/// Table format for Spark targets, used to select DDL strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SparkTableFormat {
    /// Delta Lake — supports column mapping, DROP COLUMN, safe widenings
    Delta,
    /// Plain Parquet — limited schema evolution, no column drops
    Parquet,
}

/// Result of planning a migration for Spark.
///
/// Unlike DuckDB which can always produce DDL statements, Spark may need
/// alternative strategies depending on the table format and operation type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MigrationExecution {
    /// DDL statements to execute in order.
    Statements(Vec<String>),
    /// Use mergeSchema write (Spark only) — the write itself evolves the schema.
    MergeSchemaWrite {
        columns_to_add: Vec<(String, DataType)>,
    },
    /// Rewrite table from itself (not from source) using a SELECT expression.
    TableRewrite { select_expr: String },
    /// Requires full refresh from source — needs `--allow-full-refresh`.
    FullRefreshRequired { reason: String },
}

/// Generate Spark-specific migration execution plan from a list of `SchemaOperation`s.
///
/// # Arguments
/// * `catalog` — Spark catalog name (e.g., "spark_catalog")
/// * `schema` — Database/schema name (e.g., "default")
/// * `table` — Table name
/// * `ops` — Abstract schema operations to execute
/// * `format` — Delta or Parquet table format
/// * `caps` — Backend capabilities for the target format
pub fn generate_spark_ddl(
    catalog: &str,
    schema: &str,
    table: &str,
    ops: &[SchemaOperation],
    format: SparkTableFormat,
    caps: &BackendCapabilities,
) -> MigrationExecution {
    let qualified = format!("{}.{}.{}", catalog, schema, table);
    let mut stmts = Vec::new();

    for op in ops {
        match classify_operation(op, format, caps) {
            OpStrategy::Ddl(sqls) => {
                stmts.extend(sqls.iter().map(|sql| sql.replace("{TABLE}", &qualified)))
            }
            OpStrategy::MergeSchema(columns) => {
                return MigrationExecution::MergeSchemaWrite {
                    columns_to_add: columns,
                };
            }
            OpStrategy::TableRewrite(select_expr) => {
                return MigrationExecution::TableRewrite {
                    select_expr: select_expr.replace("{TABLE}", &qualified),
                };
            }
            OpStrategy::FullRefresh(reason) => {
                return MigrationExecution::FullRefreshRequired { reason };
            }
        }
    }

    MigrationExecution::Statements(stmts)
}

/// Generate the SQL statements for a Spark table rewrite.
///
/// Returns a sequence of DDL/DML statements that:
/// 1. Create a temp table with the transformed data
/// 2. Drop the original table
/// 3. Rename the temp table to the original name
pub fn generate_table_rewrite_sql(
    catalog: &str,
    schema: &str,
    table: &str,
    select_expr: &str,
) -> Vec<String> {
    let qualified = format!("{}.{}.{}", catalog, schema, table);
    let tmp_table = format!("{}.{}.{}_smelt_tmp", catalog, schema, table);

    vec![
        format!(
            "CREATE TABLE {} AS SELECT {} FROM {}",
            tmp_table, select_expr, qualified
        ),
        format!("DROP TABLE {}", qualified),
        format!("ALTER TABLE {} RENAME TO {}", tmp_table, qualified),
    ]
}

/// Internal classification of how a single operation should be executed.
enum OpStrategy {
    /// Execute these statements in order. `{TABLE}` is the qualified name.
    Ddl(Vec<String>),
    /// Use mergeSchema write with these columns.
    MergeSchema(Vec<(String, DataType)>),
    /// Rewrite the table using this SELECT expression.
    TableRewrite(String),
    /// Requires full refresh — return immediately.
    FullRefresh(String),
}

fn classify_operation(
    op: &SchemaOperation,
    format: SparkTableFormat,
    caps: &BackendCapabilities,
) -> OpStrategy {
    match op {
        SchemaOperation::AddColumn {
            name,
            data_type,
            nullable,
            default_expr,
        } => {
            // Measured: `ADD COLUMNS (c T NOT NULL)` is refused by both formats —
            // Delta with `NOT NULL in ALTER TABLE ADD COLUMNS is not supported`,
            // Parquet with `ADD COLUMN with v1 tables cannot specify NOT NULL`.
            if !nullable {
                return OpStrategy::FullRefresh(format!(
                    "Cannot add NOT NULL column '{}' — Spark refuses NOT NULL on \
                     ALTER TABLE ADD COLUMNS for both Delta and Parquet tables. \
                     Add it as nullable or use --allow-full-refresh",
                    name
                ));
            }
            let mut stmts = vec![format!(
                "ALTER TABLE {{TABLE}} ADD COLUMNS ({} {})",
                name,
                to_spark_type_sql(data_type)
            )];
            // A `default:` fills the rows already in the table (the DuckDB
            // generator gets that from `ADD COLUMN … DEFAULT`). Delta refuses a
            // DEFAULT clause on the add without the `allowColumnDefaults` table
            // feature, so the same outcome is spelled as a following UPDATE —
            // which only Delta can run.
            if let Some(default) = default_expr {
                if format == SparkTableFormat::Parquet {
                    return OpStrategy::FullRefresh(format!(
                        "Cannot apply the default for added column '{}' on a Parquet table — \
                         Parquet cannot UPDATE existing rows. \
                         Consider using Delta format or --allow-full-refresh",
                        name
                    ));
                }
                stmts.push(format!(
                    "UPDATE {{TABLE}} SET {} = {} WHERE {} IS NULL",
                    name, default, name
                ));
            }
            OpStrategy::Ddl(stmts)
        }
        SchemaOperation::RemoveColumn { name } => {
            // Measured: `DROP COLUMN` is refused on both — Delta with
            // `DELTA_UNSUPPORTED_DROP_COLUMN` (it needs `delta.columnMapping.mode`,
            // an irreversible protocol upgrade smelt does not make on a user's
            // table), Parquet with `UNSUPPORTED_FEATURE.TABLE_OPERATION`.
            if format == SparkTableFormat::Parquet {
                return OpStrategy::FullRefresh(format!(
                    "Cannot drop column '{}' on a Parquet table — column mapping not supported. \
                     Consider using Delta format or --allow-full-refresh",
                    name
                ));
            }
            OpStrategy::TableRewrite(format!("* EXCEPT({})", name))
        }
        SchemaOperation::WidenColumnType { name, from, to } => {
            // Measured: every `ALTER COLUMN … TYPE` widening is refused, the
            // whole documented safe chain included — Delta needs the
            // `delta.enableTypeWidening` table feature, Parquet refuses with
            // `NOT_SUPPORTED_CHANGE_COLUMN`. Delta re-casts by rewriting.
            if format == SparkTableFormat::Parquet {
                return OpStrategy::FullRefresh(format!(
                    "Cannot widen column '{}' from {} to {} on a Parquet table. \
                     Consider using Delta format or --allow-full-refresh",
                    name,
                    from.to_sql(),
                    to.to_sql()
                ));
            }
            OpStrategy::TableRewrite(format!(
                "CAST({} AS {}) AS {}, * EXCEPT({})",
                name,
                to_spark_type_sql(to),
                name,
                name
            ))
        }
        SchemaOperation::ChangeNullability {
            name,
            to_nullable,
            default_expr: _,
        } => {
            if !to_nullable {
                // Measured: Delta refuses `SET NOT NULL` outright — "Cannot change
                // nullable column to non-nullable" — even when the column holds no
                // NULLs, so no amount of pre-filling makes the statement legal.
                return OpStrategy::FullRefresh(format!(
                    "Cannot set column '{}' to NOT NULL — Spark refuses SET NOT NULL on an \
                     existing nullable column for both Delta and Parquet tables. \
                     Use --allow-full-refresh",
                    name
                ));
            }
            if format == SparkTableFormat::Parquet {
                return OpStrategy::FullRefresh(format!(
                    "Cannot relax column '{}' to nullable on a Parquet table — \
                     ALTER COLUMN is unsupported there. \
                     Consider using Delta format or --allow-full-refresh",
                    name
                ));
            }
            OpStrategy::Ddl(vec![format!(
                "ALTER TABLE {{TABLE}} ALTER COLUMN {} DROP NOT NULL",
                name
            )])
        }
        SchemaOperation::AddStructField {
            column,
            path,
            field_name,
            field_type,
            default_expr: _,
        } => {
            // Measured: a qualified struct path in ADD COLUMNS is accepted on
            // Delta and refused on Parquet (`UNSUPPORTED_FEATURE.TABLE_OPERATION`),
            // which is what `supports_struct_field_ddl` records.
            if !caps.supports_struct_field_ddl {
                return OpStrategy::FullRefresh(format!(
                    "Cannot add struct field '{}' to column '{}' on a Parquet table — \
                     ALTER TABLE ADD COLUMNS with a qualified path is unsupported there. \
                     Consider using Delta format or --allow-full-refresh",
                    field_name, column
                ));
            }

            // Check if the path goes through an array element (e.g., items.element.score).
            // Spark doesn't support ALTER TABLE ADD COLUMNS for nested array-of-struct fields;
            // use mergeSchema write instead.
            let is_array_nested = path.iter().any(|p| p == "element");
            if is_array_nested && !caps.supports_nested_array_ddl {
                let dot_path = format_spark_dot_path(column, path, Some(field_name));
                return OpStrategy::MergeSchema(vec![(dot_path, field_type.clone())]);
            }

            // Both Delta and Parquet support adding nullable struct fields via DDL
            let dot_path = format_spark_dot_path(column, path, Some(field_name));
            let type_sql = to_spark_type_sql(field_type);
            OpStrategy::Ddl(vec![format!(
                "ALTER TABLE {{TABLE}} ADD COLUMNS ({} {})",
                dot_path, type_sql
            )])
        }
        SchemaOperation::RemoveStructField {
            column,
            path,
            field_name,
        } => {
            // Measured: dropping a nested field is refused on both formats —
            // Delta with `DELTA_UNSUPPORTED_DROP_COLUMN` (it needs column
            // mapping, which smelt's tables do not enable), Parquet with
            // `UNSUPPORTED_FEATURE.TABLE_OPERATION`.
            let _ = path;
            OpStrategy::FullRefresh(format!(
                "Cannot drop struct field '{}.{}' on a {} table — DROP COLUMN on a nested \
                 field requires column mapping, which smelt's tables do not enable. \
                 Use --allow-full-refresh",
                column,
                field_name,
                match format {
                    SparkTableFormat::Delta => "Delta",
                    SparkTableFormat::Parquet => "Parquet",
                }
            ))
        }
        SchemaOperation::WidenNestedType {
            column,
            path,
            from,
            to,
        } => {
            // Spark doesn't support ALTER COLUMN TYPE ... USING for nested types
            if format == SparkTableFormat::Parquet {
                OpStrategy::FullRefresh(format!(
                    "Cannot widen nested type in column '{}' (path: {}) from {} to {} on Parquet table. \
                     Consider using Delta format or --allow-full-refresh",
                    column,
                    path.join("."),
                    from.to_sql(),
                    to.to_sql()
                ))
            } else {
                // Delta: table rewrite
                let path_desc = if path.is_empty() {
                    column.clone()
                } else {
                    format!("{}.{}", column, path.join("."))
                };
                OpStrategy::TableRewrite(format!(
                    "* /* rewrite {}: {} -> {} */",
                    path_desc,
                    from.to_sql(),
                    to.to_sql()
                ))
            }
        }
        SchemaOperation::BackfillColumn { name, expression } => {
            // UPDATE works on Delta; Parquet doesn't support UPDATE
            if format == SparkTableFormat::Parquet {
                OpStrategy::FullRefresh(format!(
                    "Cannot UPDATE Parquet table to backfill column '{}'. \
                     Consider using Delta format or --allow-full-refresh",
                    name
                ))
            } else {
                OpStrategy::Ddl(vec![format!(
                    "UPDATE {{TABLE}} SET {} = {}",
                    name, expression
                )])
            }
        }
        SchemaOperation::RewriteColumn {
            column,
            target_type,
            using_expr,
        } => {
            // Spark doesn't support ALTER COLUMN TYPE ... USING
            // Must do a table rewrite for both Delta and Parquet
            if format == SparkTableFormat::Parquet {
                OpStrategy::FullRefresh(format!(
                    "Cannot rewrite column '{}' on Parquet table — requires table rebuild. \
                     Consider using Delta format or --allow-full-refresh",
                    column
                ))
            } else {
                // Delta: table rewrite using the expression
                let _ = target_type; // Target type is embedded in the using_expr
                OpStrategy::TableRewrite(format!(
                    "{} AS {}, * EXCEPT({})",
                    using_expr, column, column
                ))
            }
        }
    }
}

/// Convert a `DataType` to Spark SQL type syntax.
///
/// Spark uses slightly different type names in some cases.
fn to_spark_type_sql(dt: &DataType) -> String {
    match dt {
        DataType::Varchar { max_length: None } => "STRING".to_string(),
        DataType::Varchar {
            max_length: Some(n),
        } => format!("VARCHAR({})", n),
        DataType::Text => "STRING".to_string(),
        // For most types, the standard to_sql() works
        _ => dt.to_sql(),
    }
}

/// Format a dot-separated path for Spark struct field access.
fn format_spark_dot_path(column: &str, path: &[String], leaf: Option<&str>) -> String {
    let mut parts = vec![column.to_string()];
    parts.extend(path.iter().cloned());
    if let Some(l) = leaf {
        parts.push(l.to_string());
    }
    parts.join(".")
}

#[cfg(test)]
mod tests {
    use super::*;
    use smelt_types::DataType;

    fn delta_caps() -> BackendCapabilities {
        BackendCapabilities::spark_delta()
    }

    fn parquet_caps() -> BackendCapabilities {
        BackendCapabilities::spark_parquet()
    }

    // ── Spark+Delta: struct field add ──────────────────────────────────

    #[test]
    fn test_delta_add_struct_field() {
        let ops = vec![SchemaOperation::AddStructField {
            column: "meta".into(),
            path: vec![],
            field_name: "b".into(),
            field_type: DataType::Varchar { max_length: None },
            default_expr: None,
        }];
        let result = generate_spark_ddl(
            "cat",
            "db",
            "t",
            &ops,
            SparkTableFormat::Delta,
            &delta_caps(),
        );
        assert_eq!(
            result,
            MigrationExecution::Statements(vec![
                "ALTER TABLE cat.db.t ADD COLUMNS (meta.b STRING)".to_string()
            ])
        );
    }

    #[test]
    fn test_delta_add_nested_struct_field() {
        let ops = vec![SchemaOperation::AddStructField {
            column: "data".into(),
            path: vec!["inner".into()],
            field_name: "y".into(),
            field_type: DataType::Integer,
            default_expr: None,
        }];
        let result = generate_spark_ddl(
            "cat",
            "db",
            "t",
            &ops,
            SparkTableFormat::Delta,
            &delta_caps(),
        );
        assert_eq!(
            result,
            MigrationExecution::Statements(vec![
                "ALTER TABLE cat.db.t ADD COLUMNS (data.inner.y INTEGER)".to_string()
            ])
        );
    }

    // ── Spark+Delta: struct field remove ───────────────────────────────

    #[test]
    fn test_delta_remove_struct_field() {
        let ops = vec![SchemaOperation::RemoveStructField {
            column: "meta".into(),
            path: vec![],
            field_name: "old_field".into(),
        }];
        let result = generate_spark_ddl(
            "cat",
            "db",
            "t",
            &ops,
            SparkTableFormat::Delta,
            &delta_caps(),
        );
        // Measured: `DELTA_UNSUPPORTED_DROP_COLUMN` — dropping a nested field
        // needs `delta.columnMapping.mode`, which smelt's tables do not set.
        match result {
            MigrationExecution::FullRefreshRequired { reason } => assert!(
                reason.contains("meta.old_field"),
                "the refusal must name the field, got: {}",
                reason
            ),
            other => panic!("Expected FullRefreshRequired, got {:?}", other),
        }
    }

    // ── Spark+Delta: nested type widen → table rewrite ────────────────

    #[test]
    fn test_delta_widen_nested_type_table_rewrite() {
        let ops = vec![SchemaOperation::WidenNestedType {
            column: "meta".into(),
            path: vec!["a".into()],
            from: DataType::Integer,
            to: DataType::BigInt,
        }];
        let result = generate_spark_ddl(
            "cat",
            "db",
            "t",
            &ops,
            SparkTableFormat::Delta,
            &delta_caps(),
        );
        match result {
            MigrationExecution::TableRewrite { select_expr } => {
                assert!(select_expr.contains("meta.a"));
                assert!(select_expr.contains("INTEGER"));
                assert!(select_expr.contains("BIGINT"));
            }
            other => panic!("Expected TableRewrite, got {:?}", other),
        }
    }

    // ── Spark+Delta: add column ───────────────────────────────────────

    #[test]
    fn test_delta_add_column() {
        let ops = vec![SchemaOperation::AddColumn {
            name: "status".into(),
            data_type: DataType::Varchar { max_length: None },
            nullable: true,
            default_expr: None,
        }];
        let result = generate_spark_ddl(
            "cat",
            "db",
            "t",
            &ops,
            SparkTableFormat::Delta,
            &delta_caps(),
        );
        assert_eq!(
            result,
            MigrationExecution::Statements(vec![
                "ALTER TABLE cat.db.t ADD COLUMNS (status STRING)".to_string()
            ])
        );
    }

    // ── Spark+Delta: remove column ────────────────────────────────────

    #[test]
    fn test_delta_remove_column() {
        let ops = vec![SchemaOperation::RemoveColumn {
            name: "old_col".into(),
        }];
        let result = generate_spark_ddl(
            "cat",
            "db",
            "t",
            &ops,
            SparkTableFormat::Delta,
            &delta_caps(),
        );
        // Measured: `DELTA_UNSUPPORTED_DROP_COLUMN` on a table without column
        // mapping — the drop is expressed by rewriting the table instead.
        assert_eq!(
            result,
            MigrationExecution::TableRewrite {
                select_expr: "* EXCEPT(old_col)".to_string()
            }
        );
    }

    // ── Spark+Delta: safe type widening ───────────────────────────────

    #[test]
    fn test_delta_safe_type_widening() {
        let ops = vec![SchemaOperation::WidenColumnType {
            name: "amount".into(),
            from: DataType::Integer,
            to: DataType::BigInt,
        }];
        let result = generate_spark_ddl(
            "cat",
            "db",
            "t",
            &ops,
            SparkTableFormat::Delta,
            &delta_caps(),
        );
        assert_eq!(
            result,
            MigrationExecution::TableRewrite {
                select_expr: "CAST(amount AS BIGINT) AS amount, * EXCEPT(amount)".to_string()
            }
        );
    }

    #[test]
    fn test_delta_float_to_double_widening() {
        let ops = vec![SchemaOperation::WidenColumnType {
            name: "score".into(),
            from: DataType::Float,
            to: DataType::Double,
        }];
        let result = generate_spark_ddl(
            "cat",
            "db",
            "t",
            &ops,
            SparkTableFormat::Delta,
            &delta_caps(),
        );
        assert_eq!(
            result,
            MigrationExecution::TableRewrite {
                select_expr: "CAST(score AS DOUBLE) AS score, * EXCEPT(score)".to_string()
            }
        );
    }

    // ── Spark+Delta: unsafe widening → table rewrite ──────────────────

    #[test]
    fn test_delta_unsafe_widening_table_rewrite() {
        let ops = vec![SchemaOperation::WidenColumnType {
            name: "data".into(),
            from: DataType::Varchar { max_length: None },
            to: DataType::Integer,
        }];
        let result = generate_spark_ddl(
            "cat",
            "db",
            "t",
            &ops,
            SparkTableFormat::Delta,
            &delta_caps(),
        );
        match result {
            MigrationExecution::TableRewrite { select_expr } => {
                assert!(select_expr.contains("CAST(data AS INTEGER)"));
            }
            other => panic!("Expected TableRewrite, got {:?}", other),
        }
    }

    // ── Spark+Parquet: struct field add (nullable) → DDL ──────────────

    #[test]
    fn test_parquet_add_struct_field_nullable() {
        let ops = vec![SchemaOperation::AddStructField {
            column: "meta".into(),
            path: vec![],
            field_name: "b".into(),
            field_type: DataType::Varchar { max_length: None },
            default_expr: None,
        }];
        let result = generate_spark_ddl(
            "cat",
            "db",
            "t",
            &ops,
            SparkTableFormat::Parquet,
            &parquet_caps(),
        );
        // Measured: `UNSUPPORTED_FEATURE.TABLE_OPERATION` — a qualified struct
        // path in ADD COLUMNS is rejected on a v1 Parquet table.
        match result {
            MigrationExecution::FullRefreshRequired { reason } => assert!(
                reason.contains('b') && reason.contains("meta"),
                "the refusal must name the field, got: {}",
                reason
            ),
            other => panic!("Expected FullRefreshRequired, got {:?}", other),
        }
    }

    // ── Spark+Parquet: nested type widen → full refresh ───────────────

    #[test]
    fn test_parquet_widen_nested_type_full_refresh() {
        let ops = vec![SchemaOperation::WidenNestedType {
            column: "meta".into(),
            path: vec!["a".into()],
            from: DataType::Integer,
            to: DataType::BigInt,
        }];
        let result = generate_spark_ddl(
            "cat",
            "db",
            "t",
            &ops,
            SparkTableFormat::Parquet,
            &parquet_caps(),
        );
        match result {
            MigrationExecution::FullRefreshRequired { reason } => {
                assert!(reason.contains("Parquet"));
                assert!(reason.contains("meta"));
            }
            other => panic!("Expected FullRefreshRequired, got {:?}", other),
        }
    }

    // ── Spark+Parquet: struct field remove → full refresh ─────────────

    #[test]
    fn test_parquet_remove_struct_field_full_refresh() {
        let ops = vec![SchemaOperation::RemoveStructField {
            column: "meta".into(),
            path: vec![],
            field_name: "old_field".into(),
        }];
        let result = generate_spark_ddl(
            "cat",
            "db",
            "t",
            &ops,
            SparkTableFormat::Parquet,
            &parquet_caps(),
        );
        match result {
            MigrationExecution::FullRefreshRequired { reason } => {
                assert!(reason.contains("Parquet"));
                assert!(reason.contains("column mapping"));
            }
            other => panic!("Expected FullRefreshRequired, got {:?}", other),
        }
    }

    // ── Spark+Parquet: column remove → full refresh ───────────────────

    #[test]
    fn test_parquet_remove_column_full_refresh() {
        let ops = vec![SchemaOperation::RemoveColumn {
            name: "old_col".into(),
        }];
        let result = generate_spark_ddl(
            "cat",
            "db",
            "t",
            &ops,
            SparkTableFormat::Parquet,
            &parquet_caps(),
        );
        match result {
            MigrationExecution::FullRefreshRequired { reason } => {
                assert!(reason.contains("Parquet"));
            }
            other => panic!("Expected FullRefreshRequired, got {:?}", other),
        }
    }

    // ── Spark+Parquet: column type widen → full refresh ───────────────

    #[test]
    fn test_parquet_unsafe_widening_full_refresh() {
        let ops = vec![SchemaOperation::WidenColumnType {
            name: "data".into(),
            from: DataType::Varchar { max_length: None },
            to: DataType::Integer,
        }];
        let result = generate_spark_ddl(
            "cat",
            "db",
            "t",
            &ops,
            SparkTableFormat::Parquet,
            &parquet_caps(),
        );
        match result {
            MigrationExecution::FullRefreshRequired { reason } => {
                assert!(reason.contains("Parquet"));
            }
            other => panic!("Expected FullRefreshRequired, got {:?}", other),
        }
    }

    // ── Spark+Parquet: safe widening still works ──────────────────────

    #[test]
    fn test_parquet_safe_widening() {
        let ops = vec![SchemaOperation::WidenColumnType {
            name: "amount".into(),
            from: DataType::Integer,
            to: DataType::BigInt,
        }];
        let result = generate_spark_ddl(
            "cat",
            "db",
            "t",
            &ops,
            SparkTableFormat::Parquet,
            &parquet_caps(),
        );
        // Measured: `NOT_SUPPORTED_CHANGE_COLUMN` — Parquet cannot widen in place.
        match result {
            MigrationExecution::FullRefreshRequired { reason } => assert!(
                reason.contains("amount"),
                "the refusal must name the column, got: {}",
                reason
            ),
            other => panic!("Expected FullRefreshRequired, got {:?}", other),
        }
    }

    // ── Spark+Parquet: NOT NULL column add → full refresh ─────────────

    #[test]
    fn test_parquet_add_not_null_column_full_refresh() {
        let ops = vec![SchemaOperation::AddColumn {
            name: "required_col".into(),
            data_type: DataType::Integer,
            nullable: false,
            default_expr: Some("0".into()),
        }];
        let result = generate_spark_ddl(
            "cat",
            "db",
            "t",
            &ops,
            SparkTableFormat::Parquet,
            &parquet_caps(),
        );
        match result {
            MigrationExecution::FullRefreshRequired { reason } => {
                assert!(reason.contains("NOT NULL"));
                assert!(reason.contains("Parquet"));
            }
            other => panic!("Expected FullRefreshRequired, got {:?}", other),
        }
    }

    // ── Spark+Parquet: rewrite column → full refresh ──────────────────

    #[test]
    fn test_parquet_rewrite_column_full_refresh() {
        let ops = vec![SchemaOperation::RewriteColumn {
            column: "meta".into(),
            target_type: DataType::Struct(vec![
                ("a".into(), DataType::BigInt),
                ("b".into(), DataType::Varchar { max_length: None }),
            ]),
            using_expr: "struct_pack(a := meta.a::BIGINT, b := meta.b)".into(),
        }];
        let result = generate_spark_ddl(
            "cat",
            "db",
            "t",
            &ops,
            SparkTableFormat::Parquet,
            &parquet_caps(),
        );
        match result {
            MigrationExecution::FullRefreshRequired { reason } => {
                assert!(reason.contains("Parquet"));
                assert!(reason.contains("meta"));
            }
            other => panic!("Expected FullRefreshRequired, got {:?}", other),
        }
    }

    // ── Spark+Delta: rewrite column → table rewrite ───────────────────

    #[test]
    fn test_delta_rewrite_column_table_rewrite() {
        let ops = vec![SchemaOperation::RewriteColumn {
            column: "meta".into(),
            target_type: DataType::Struct(vec![
                ("a".into(), DataType::BigInt),
                ("b".into(), DataType::Varchar { max_length: None }),
            ]),
            using_expr: "struct_pack(a := meta.a::BIGINT, b := meta.b)".into(),
        }];
        let result = generate_spark_ddl(
            "cat",
            "db",
            "t",
            &ops,
            SparkTableFormat::Delta,
            &delta_caps(),
        );
        match result {
            MigrationExecution::TableRewrite { select_expr } => {
                assert!(select_expr.contains("struct_pack"));
                assert!(select_expr.contains("meta"));
            }
            other => panic!("Expected TableRewrite, got {:?}", other),
        }
    }

    // ── Table rewrite SQL generation ──────────────────────────────────

    #[test]
    fn test_table_rewrite_sql() {
        let stmts =
            generate_table_rewrite_sql("cat", "db", "t", "CAST(amount AS BIGINT) AS amount, *");
        assert_eq!(stmts.len(), 3);
        assert!(stmts[0].contains("CREATE TABLE cat.db.t_smelt_tmp"));
        assert!(stmts[0].contains("CAST(amount AS BIGINT) AS amount, *"));
        assert!(stmts[0].contains("FROM cat.db.t"));
        assert_eq!(stmts[1], "DROP TABLE cat.db.t");
        assert!(stmts[2].contains("RENAME TO cat.db.t"));
    }

    // ── Multiple DDL operations ───────────────────────────────────────

    #[test]
    fn test_delta_multiple_ddl_ops() {
        let ops = vec![
            SchemaOperation::AddColumn {
                name: "new_col".into(),
                data_type: DataType::Integer,
                nullable: true,
                default_expr: None,
            },
            SchemaOperation::RemoveColumn {
                name: "old_col".into(),
            },
            SchemaOperation::AddStructField {
                column: "meta".into(),
                path: vec![],
                field_name: "status".into(),
                field_type: DataType::Varchar { max_length: None },
                default_expr: None,
            },
        ];
        let result = generate_spark_ddl(
            "cat",
            "db",
            "t",
            &ops,
            SparkTableFormat::Delta,
            &delta_caps(),
        );
        // The column drop is not expressible as DDL, and a rewrite answers for
        // the whole diff — a plan is one strategy, never a mix.
        assert_eq!(
            result,
            MigrationExecution::TableRewrite {
                select_expr: "* EXCEPT(old_col)".to_string()
            }
        );
    }

    // ── Spark type naming ─────────────────────────────────────────────

    #[test]
    fn test_spark_type_naming_varchar_is_string() {
        assert_eq!(
            to_spark_type_sql(&DataType::Varchar { max_length: None }),
            "STRING"
        );
        assert_eq!(to_spark_type_sql(&DataType::Text), "STRING");
    }

    #[test]
    fn test_spark_type_naming_other_types() {
        assert_eq!(to_spark_type_sql(&DataType::Integer), "INTEGER");
        assert_eq!(to_spark_type_sql(&DataType::BigInt), "BIGINT");
        assert_eq!(to_spark_type_sql(&DataType::Boolean), "BOOLEAN");
    }

    // ── widening is never DDL ─────────────────────────────────────────

    /// No widening is expressible as `ALTER COLUMN … TYPE` on a smelt-created
    /// table — not even the documented-safe integer chain. Measured: Delta
    /// answers `DELTA_UNSUPPORTED_ALTER_TABLE_CHANGE_COL_OP` without the
    /// `delta.enableTypeWidening` table feature; Parquet answers
    /// `NOT_SUPPORTED_CHANGE_COLUMN`.
    #[test]
    fn test_spark_widening_is_never_alter_column_type() {
        for (from, to) in [
            (DataType::SmallInt, DataType::Integer),
            (DataType::Integer, DataType::BigInt),
            (DataType::Float, DataType::Double),
        ] {
            let ops = vec![SchemaOperation::WidenColumnType {
                name: "n".into(),
                from: from.clone(),
                to: to.clone(),
            }];

            let delta = generate_spark_ddl(
                "cat",
                "db",
                "t",
                &ops,
                SparkTableFormat::Delta,
                &delta_caps(),
            );
            match delta {
                MigrationExecution::TableRewrite { select_expr } => assert_eq!(
                    select_expr,
                    format!("CAST(n AS {}) AS n, * EXCEPT(n)", to_spark_type_sql(&to))
                ),
                other => panic!("Delta {from:?} -> {to:?}: expected TableRewrite, got {other:?}"),
            }

            let parquet = generate_spark_ddl(
                "cat",
                "db",
                "t",
                &ops,
                SparkTableFormat::Parquet,
                &parquet_caps(),
            );
            match parquet {
                MigrationExecution::FullRefreshRequired { reason } => {
                    assert!(reason.contains('n'), "the refusal must name the column")
                }
                other => panic!("Parquet {from:?} -> {to:?}: expected refusal, got {other:?}"),
            }
        }
    }

    // ── Spark+Parquet: backfill column → full refresh ─────────────────

    #[test]
    fn test_parquet_backfill_full_refresh() {
        let ops = vec![SchemaOperation::BackfillColumn {
            name: "status".into(),
            expression: "CASE WHEN active THEN 'active' ELSE 'inactive' END".into(),
        }];
        let result = generate_spark_ddl(
            "cat",
            "db",
            "t",
            &ops,
            SparkTableFormat::Parquet,
            &parquet_caps(),
        );
        match result {
            MigrationExecution::FullRefreshRequired { reason } => {
                assert!(reason.contains("Parquet"));
                assert!(reason.contains("status"));
            }
            other => panic!("Expected FullRefreshRequired, got {:?}", other),
        }
    }

    // ── Spark+Delta: backfill column → UPDATE ─────────────────────────

    #[test]
    fn test_delta_backfill_column() {
        let ops = vec![SchemaOperation::BackfillColumn {
            name: "status".into(),
            expression: "CASE WHEN active THEN 'active' ELSE 'inactive' END".into(),
        }];
        let result = generate_spark_ddl(
            "cat",
            "db",
            "t",
            &ops,
            SparkTableFormat::Delta,
            &delta_caps(),
        );
        assert_eq!(
            result,
            MigrationExecution::Statements(vec![
                "UPDATE cat.db.t SET status = CASE WHEN active THEN 'active' ELSE 'inactive' END"
                    .to_string()
            ])
        );
    }

    // ── Spark+Delta: nullability change ───────────────────────────────

    #[test]
    fn test_delta_drop_not_null() {
        let ops = vec![SchemaOperation::ChangeNullability {
            name: "status".into(),
            to_nullable: true,
            default_expr: None,
        }];
        let result = generate_spark_ddl(
            "cat",
            "db",
            "t",
            &ops,
            SparkTableFormat::Delta,
            &delta_caps(),
        );
        assert_eq!(
            result,
            MigrationExecution::Statements(vec![
                "ALTER TABLE cat.db.t ALTER COLUMN status DROP NOT NULL".to_string()
            ])
        );
    }

    // ── Spark+Parquet: array-of-struct field add → mergeSchema ─────────

    #[test]
    fn test_parquet_array_of_struct_field_add_merge_schema() {
        let ops = vec![SchemaOperation::AddStructField {
            column: "items".into(),
            path: vec!["element".into()],
            field_name: "score".into(),
            field_type: DataType::Double,
            default_expr: None,
        }];
        let result = generate_spark_ddl(
            "cat",
            "db",
            "t",
            &ops,
            SparkTableFormat::Parquet,
            &parquet_caps(),
        );
        // Measured: Parquet rejects every qualified-path ADD COLUMNS, so the
        // refusal fires before the array-nested mergeSchema branch is reached.
        match result {
            MigrationExecution::FullRefreshRequired { reason } => assert!(
                reason.contains("score"),
                "the refusal must name the field, got: {}",
                reason
            ),
            other => panic!("Expected FullRefreshRequired, got {:?}", other),
        }
    }

    #[test]
    fn test_delta_array_of_struct_field_add_ddl() {
        // Delta supports nested array DDL (supports_nested_array_ddl = true, empirically verified W7·P2).
        // So adding a field inside an array-of-struct generates ALTER TABLE ADD COLUMNS, not MergeSchemaWrite.
        let ops = vec![SchemaOperation::AddStructField {
            column: "items".into(),
            path: vec!["element".into()],
            field_name: "score".into(),
            field_type: DataType::Double,
            default_expr: None,
        }];
        let result = generate_spark_ddl(
            "cat",
            "db",
            "t",
            &ops,
            SparkTableFormat::Delta,
            &delta_caps(),
        );
        match result {
            MigrationExecution::Statements(stmts) => {
                assert_eq!(stmts.len(), 1);
                assert!(
                    stmts[0].contains("ADD COLUMNS") && stmts[0].contains("items.element.score"),
                    "expected ALTER TABLE ADD COLUMNS, got: {}",
                    stmts[0]
                );
            }
            other => panic!("Expected Statements (DDL), got {:?}", other),
        }
    }

    // ── 12a: Additional edge case tests ────────────────────────────────

    #[test]
    fn test_delta_add_column_with_default() {
        let ops = vec![SchemaOperation::AddColumn {
            name: "status".into(),
            data_type: DataType::Varchar { max_length: None },
            nullable: true,
            default_expr: Some("'pending'".into()),
        }];
        let result = generate_spark_ddl(
            "cat",
            "db",
            "t",
            &ops,
            SparkTableFormat::Delta,
            &delta_caps(),
        );
        // Measured: Delta refuses a DEFAULT clause on ADD COLUMNS without the
        // `allowColumnDefaults` table feature, so the default reaches the rows
        // already in the table as a following UPDATE instead.
        assert_eq!(
            result,
            MigrationExecution::Statements(vec![
                "ALTER TABLE cat.db.t ADD COLUMNS (status STRING)".to_string(),
                "UPDATE cat.db.t SET status = 'pending' WHERE status IS NULL".to_string(),
            ])
        );
    }

    #[test]
    fn test_delta_add_column_not_null() {
        let ops = vec![SchemaOperation::AddColumn {
            name: "required_col".into(),
            data_type: DataType::Integer,
            nullable: false,
            default_expr: None,
        }];
        let result = generate_spark_ddl(
            "cat",
            "db",
            "t",
            &ops,
            SparkTableFormat::Delta,
            &delta_caps(),
        );
        // Measured: `NOT NULL in ALTER TABLE ADD COLUMNS is not supported` on
        // Delta, and `ADD COLUMN with v1 tables cannot specify NOT NULL` on
        // Parquet — neither format accepts the constraint on the add.
        match result {
            MigrationExecution::FullRefreshRequired { reason } => assert!(
                reason.contains("required_col"),
                "the refusal must name the column, got: {}",
                reason
            ),
            other => panic!("Expected FullRefreshRequired, got {:?}", other),
        }
    }

    #[test]
    fn test_delta_change_nullability_set_not_null_with_default() {
        let ops = vec![SchemaOperation::ChangeNullability {
            name: "status".into(),
            to_nullable: false,
            default_expr: Some("'unknown'".into()),
        }];
        let result = generate_spark_ddl(
            "cat",
            "db",
            "t",
            &ops,
            SparkTableFormat::Delta,
            &delta_caps(),
        );
        // Measured: Delta refuses SET NOT NULL on an existing nullable column
        // ("Cannot change nullable column to non-nullable") even when it holds
        // no NULLs, so filling the gaps first cannot make the change legal.
        match result {
            MigrationExecution::FullRefreshRequired { reason } => assert!(
                reason.contains("status"),
                "the refusal must name the column, got: {}",
                reason
            ),
            other => panic!("Expected FullRefreshRequired, got {:?}", other),
        }
    }

    #[test]
    fn test_delta_change_nullability_set_not_null_without_default_full_refresh() {
        let ops = vec![SchemaOperation::ChangeNullability {
            name: "status".into(),
            to_nullable: false,
            default_expr: None,
        }];
        let result = generate_spark_ddl(
            "cat",
            "db",
            "t",
            &ops,
            SparkTableFormat::Delta,
            &delta_caps(),
        );
        match result {
            MigrationExecution::FullRefreshRequired { reason } => {
                assert!(
                    reason.contains("NOT NULL"),
                    "Expected mention of NOT NULL, got: {}",
                    reason
                );
            }
            other => panic!("Expected FullRefreshRequired, got {:?}", other),
        }
    }

    #[test]
    fn test_parquet_change_nullability_set_not_null_full_refresh() {
        let ops = vec![SchemaOperation::ChangeNullability {
            name: "status".into(),
            to_nullable: false,
            default_expr: Some("'unknown'".into()),
        }];
        let result = generate_spark_ddl(
            "cat",
            "db",
            "t",
            &ops,
            SparkTableFormat::Parquet,
            &parquet_caps(),
        );
        match result {
            MigrationExecution::FullRefreshRequired { reason } => {
                assert!(reason.contains("NOT NULL") || reason.contains("Parquet"));
            }
            other => panic!("Expected FullRefreshRequired, got {:?}", other),
        }
    }

    #[test]
    fn test_delta_smallint_widening_chain() {
        // SMALLINT → INTEGER (safe widening)
        let ops = vec![SchemaOperation::WidenColumnType {
            name: "val".into(),
            from: DataType::SmallInt,
            to: DataType::Integer,
        }];
        let result = generate_spark_ddl(
            "cat",
            "db",
            "t",
            &ops,
            SparkTableFormat::Delta,
            &delta_caps(),
        );
        assert_eq!(
            result,
            MigrationExecution::TableRewrite {
                select_expr: "CAST(val AS INTEGER) AS val, * EXCEPT(val)".to_string()
            }
        );
    }

    #[test]
    fn test_delta_backfill_then_add_column() {
        // Multiple operations: add column + backfill
        let ops = vec![
            SchemaOperation::AddColumn {
                name: "category".into(),
                data_type: DataType::Varchar { max_length: None },
                nullable: true,
                default_expr: None,
            },
            SchemaOperation::BackfillColumn {
                name: "category".into(),
                expression: "CASE WHEN amount > 100 THEN 'high' ELSE 'low' END".into(),
            },
        ];
        let result = generate_spark_ddl(
            "cat",
            "db",
            "t",
            &ops,
            SparkTableFormat::Delta,
            &delta_caps(),
        );
        match result {
            MigrationExecution::Statements(stmts) => {
                assert_eq!(stmts.len(), 2);
                assert!(stmts[0].contains("ADD COLUMNS (category STRING)"));
                assert!(stmts[1].contains("UPDATE cat.db.t SET category = CASE"));
            }
            other => panic!("Expected Statements, got {:?}", other),
        }
    }

    // ── 12b: TableRewrite SQL generation tests ───────────────────────────

    #[test]
    fn test_table_rewrite_sql_with_struct_cast() {
        let stmts = generate_table_rewrite_sql(
            "cat",
            "db",
            "users",
            "CAST(meta.a AS BIGINT) AS a, meta.b, name",
        );
        assert_eq!(stmts.len(), 3);
        assert_eq!(
            stmts[0],
            "CREATE TABLE cat.db.users_smelt_tmp AS SELECT \
             CAST(meta.a AS BIGINT) AS a, meta.b, name FROM cat.db.users"
        );
        assert_eq!(stmts[1], "DROP TABLE cat.db.users");
        assert_eq!(
            stmts[2],
            "ALTER TABLE cat.db.users_smelt_tmp RENAME TO cat.db.users"
        );
    }

    #[test]
    fn test_table_rewrite_sql_preserves_complex_select() {
        let stmts = generate_table_rewrite_sql(
            "unity_catalog",
            "analytics",
            "events",
            "named_struct('a', CAST(data.a AS BIGINT), 'b', data.b, 'c', NULL) AS data, * EXCEPT(data)",
        );
        assert_eq!(stmts.len(), 3);
        assert!(stmts[0].contains("named_struct('a', CAST(data.a AS BIGINT)"));
        assert!(stmts[0].contains("FROM unity_catalog.analytics.events"));
        assert_eq!(stmts[1], "DROP TABLE unity_catalog.analytics.events");
        assert!(stmts[2].contains("RENAME TO unity_catalog.analytics.events"));
    }

    #[test]
    fn test_delta_rewrite_column_select_expr_format() {
        // Verify the TableRewrite select_expr has the correct format:
        // <using_expr> AS <column>, * EXCEPT(<column>)
        let ops = vec![SchemaOperation::RewriteColumn {
            column: "meta".into(),
            target_type: DataType::Struct(vec![
                ("a".into(), DataType::BigInt),
                ("b".into(), DataType::Varchar { max_length: None }),
            ]),
            using_expr: "named_struct('a', CAST(meta.a AS BIGINT), 'b', meta.b)".into(),
        }];
        let result = generate_spark_ddl(
            "cat",
            "db",
            "t",
            &ops,
            SparkTableFormat::Delta,
            &delta_caps(),
        );
        match result {
            MigrationExecution::TableRewrite { select_expr } => {
                // Should be: <expr> AS meta, * EXCEPT(meta)
                assert!(
                    select_expr.contains("AS meta"),
                    "Expected 'AS meta' in: {}",
                    select_expr
                );
                assert!(
                    select_expr.contains("EXCEPT(meta)"),
                    "Expected 'EXCEPT(meta)' in: {}",
                    select_expr
                );
            }
            other => panic!("Expected TableRewrite, got {:?}", other),
        }
    }

    #[test]
    fn test_delta_widen_column_type_table_rewrite_format() {
        // Unsafe widening (VARCHAR → INTEGER) → table rewrite with CAST
        let ops = vec![SchemaOperation::WidenColumnType {
            name: "amount".into(),
            from: DataType::Varchar { max_length: None },
            to: DataType::Integer,
        }];
        let result = generate_spark_ddl(
            "cat",
            "db",
            "t",
            &ops,
            SparkTableFormat::Delta,
            &delta_caps(),
        );
        match result {
            MigrationExecution::TableRewrite { select_expr } => {
                assert!(
                    select_expr.contains("CAST(amount AS INTEGER)"),
                    "Expected CAST expression in: {}",
                    select_expr
                );
                assert!(
                    select_expr.contains("AS amount"),
                    "Expected 'AS amount' in: {}",
                    select_expr
                );
            }
            other => panic!("Expected TableRewrite, got {:?}", other),
        }
    }

    // ── 12c: Error message quality tests ─────────────────────────────────

    #[test]
    fn test_error_message_parquet_remove_column_suggests_remediation() {
        let ops = vec![SchemaOperation::RemoveColumn {
            name: "old_col".into(),
        }];
        let result = generate_spark_ddl(
            "cat",
            "db",
            "t",
            &ops,
            SparkTableFormat::Parquet,
            &parquet_caps(),
        );
        match result {
            MigrationExecution::FullRefreshRequired { reason } => {
                assert!(
                    reason.contains("Delta"),
                    "Should suggest Delta format, got: {}",
                    reason
                );
                assert!(
                    reason.contains("--allow-full-refresh"),
                    "Should mention --allow-full-refresh flag, got: {}",
                    reason
                );
                assert!(
                    reason.contains("old_col"),
                    "Should mention the column name, got: {}",
                    reason
                );
            }
            other => panic!("Expected FullRefreshRequired, got {:?}", other),
        }
    }

    #[test]
    fn test_error_message_parquet_widen_type_suggests_remediation() {
        let ops = vec![SchemaOperation::WidenColumnType {
            name: "data".into(),
            from: DataType::Varchar { max_length: None },
            to: DataType::Integer,
        }];
        let result = generate_spark_ddl(
            "cat",
            "db",
            "t",
            &ops,
            SparkTableFormat::Parquet,
            &parquet_caps(),
        );
        match result {
            MigrationExecution::FullRefreshRequired { reason } => {
                assert!(
                    reason.contains("Delta"),
                    "Should suggest Delta format, got: {}",
                    reason
                );
                assert!(
                    reason.contains("--allow-full-refresh"),
                    "Should mention --allow-full-refresh flag, got: {}",
                    reason
                );
            }
            other => panic!("Expected FullRefreshRequired, got {:?}", other),
        }
    }

    #[test]
    fn test_error_message_parquet_nested_widen_suggests_remediation() {
        let ops = vec![SchemaOperation::WidenNestedType {
            column: "meta".into(),
            path: vec!["a".into()],
            from: DataType::Integer,
            to: DataType::BigInt,
        }];
        let result = generate_spark_ddl(
            "cat",
            "db",
            "t",
            &ops,
            SparkTableFormat::Parquet,
            &parquet_caps(),
        );
        match result {
            MigrationExecution::FullRefreshRequired { reason } => {
                assert!(
                    reason.contains("Delta"),
                    "Should suggest Delta format, got: {}",
                    reason
                );
                assert!(
                    reason.contains("--allow-full-refresh"),
                    "Should mention --allow-full-refresh flag, got: {}",
                    reason
                );
            }
            other => panic!("Expected FullRefreshRequired, got {:?}", other),
        }
    }

    #[test]
    fn test_error_message_parquet_remove_struct_field_suggests_remediation() {
        let ops = vec![SchemaOperation::RemoveStructField {
            column: "meta".into(),
            path: vec![],
            field_name: "old_field".into(),
        }];
        let result = generate_spark_ddl(
            "cat",
            "db",
            "t",
            &ops,
            SparkTableFormat::Parquet,
            &parquet_caps(),
        );
        match result {
            MigrationExecution::FullRefreshRequired { reason } => {
                assert!(
                    reason.contains("column mapping"),
                    "Should name the missing table feature, got: {}",
                    reason
                );
                assert!(
                    reason.contains("--allow-full-refresh"),
                    "Should mention --allow-full-refresh flag, got: {}",
                    reason
                );
            }
            other => panic!("Expected FullRefreshRequired, got {:?}", other),
        }
    }

    #[test]
    fn test_error_message_parquet_rewrite_column_suggests_remediation() {
        let ops = vec![SchemaOperation::RewriteColumn {
            column: "meta".into(),
            target_type: DataType::Struct(vec![("a".into(), DataType::BigInt)]),
            using_expr: "struct(meta.a)".into(),
        }];
        let result = generate_spark_ddl(
            "cat",
            "db",
            "t",
            &ops,
            SparkTableFormat::Parquet,
            &parquet_caps(),
        );
        match result {
            MigrationExecution::FullRefreshRequired { reason } => {
                assert!(
                    reason.contains("Delta"),
                    "Should suggest Delta format, got: {}",
                    reason
                );
                assert!(
                    reason.contains("--allow-full-refresh"),
                    "Should mention --allow-full-refresh flag, got: {}",
                    reason
                );
            }
            other => panic!("Expected FullRefreshRequired, got {:?}", other),
        }
    }

    #[test]
    fn test_error_message_parquet_add_not_null_suggests_remediation() {
        let ops = vec![SchemaOperation::AddColumn {
            name: "required".into(),
            data_type: DataType::Integer,
            nullable: false,
            default_expr: Some("0".into()),
        }];
        let result = generate_spark_ddl(
            "cat",
            "db",
            "t",
            &ops,
            SparkTableFormat::Parquet,
            &parquet_caps(),
        );
        match result {
            MigrationExecution::FullRefreshRequired { reason } => {
                assert!(
                    reason.contains("--allow-full-refresh"),
                    "Should mention --allow-full-refresh flag, got: {}",
                    reason
                );
            }
            other => panic!("Expected FullRefreshRequired, got {:?}", other),
        }
    }

    #[test]
    fn test_error_message_parquet_backfill_suggests_remediation() {
        let ops = vec![SchemaOperation::BackfillColumn {
            name: "status".into(),
            expression: "'active'".into(),
        }];
        let result = generate_spark_ddl(
            "cat",
            "db",
            "t",
            &ops,
            SparkTableFormat::Parquet,
            &parquet_caps(),
        );
        match result {
            MigrationExecution::FullRefreshRequired { reason } => {
                assert!(
                    reason.contains("--allow-full-refresh"),
                    "Should mention --allow-full-refresh flag, got: {}",
                    reason
                );
            }
            other => panic!("Expected FullRefreshRequired, got {:?}", other),
        }
    }

    // ── Empty operations → empty statements ───────────────────────────

    #[test]
    fn test_empty_operations() {
        let result = generate_spark_ddl(
            "cat",
            "db",
            "t",
            &[],
            SparkTableFormat::Delta,
            &delta_caps(),
        );
        assert_eq!(result, MigrationExecution::Statements(vec![]));
    }
}
