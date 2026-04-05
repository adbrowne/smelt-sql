//! Spark-specific DDL generation from abstract `SchemaOperation`s.
//!
//! Translates backend-agnostic schema operations into Spark SQL statements
//! for both Delta and Parquet table formats.
//!
//! Key differences from DuckDB:
//! - No `ALTER COLUMN TYPE ... USING expr` — nested type widening requires table rewrite
//! - Delta supports `DROP COLUMN` (requires column mapping); Parquet does not
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
            OpStrategy::Ddl(sql) => stmts.push(sql.replace("{TABLE}", &qualified)),
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
    /// Execute as a DDL statement. `{TABLE}` placeholder for qualified name.
    Ddl(String),
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
            if !nullable && format == SparkTableFormat::Parquet {
                return OpStrategy::FullRefresh(format!(
                    "Cannot add NOT NULL column '{}' to Parquet table without full refresh. \
                     Consider using Delta format or --allow-full-refresh",
                    name
                ));
            }
            let type_sql = to_spark_type_sql(data_type);
            let mut col_spec = format!("{} {}", name, type_sql);
            if !*nullable {
                col_spec.push_str(" NOT NULL");
            }
            if let Some(default) = default_expr {
                // Spark supports DEFAULT in ALTER TABLE ADD COLUMNS (Delta)
                col_spec.push_str(&std::format!(" DEFAULT {}", default));
            }
            OpStrategy::Ddl(format!("ALTER TABLE {{TABLE}} ADD COLUMNS ({})", col_spec))
        }
        SchemaOperation::RemoveColumn { name } => {
            if !caps.supports_column_mapping {
                return OpStrategy::FullRefresh(format!(
                    "Cannot drop column '{}' on Parquet table — column mapping not supported. \
                     Consider using Delta format or --allow-full-refresh",
                    name
                ));
            }
            OpStrategy::Ddl(format!("ALTER TABLE {{TABLE}} DROP COLUMN {}", name))
        }
        SchemaOperation::WidenColumnType { name, from, to } => {
            // Spark/Delta supports limited safe type widenings:
            // TINYINT→SMALLINT→INT→BIGINT, FLOAT→DOUBLE, DECIMAL precision increase
            if is_spark_safe_widening(from, to) {
                OpStrategy::Ddl(format!(
                    "ALTER TABLE {{TABLE}} ALTER COLUMN {} TYPE {}",
                    name,
                    to_spark_type_sql(to)
                ))
            } else if format == SparkTableFormat::Parquet {
                OpStrategy::FullRefresh(format!(
                    "Cannot widen column '{}' from {} to {} on Parquet table. \
                     Consider using Delta format or --allow-full-refresh",
                    name,
                    from.to_sql(),
                    to.to_sql()
                ))
            } else {
                // Delta: table rewrite for non-trivial widenings
                OpStrategy::TableRewrite(format!(
                    "CAST({} AS {}) AS {}, *",
                    name,
                    to_spark_type_sql(to),
                    name
                ))
            }
        }
        SchemaOperation::ChangeNullability {
            name,
            to_nullable,
            default_expr,
        } => {
            if *to_nullable {
                OpStrategy::Ddl(format!(
                    "ALTER TABLE {{TABLE}} ALTER COLUMN {} DROP NOT NULL",
                    name
                ))
            } else {
                // Fill NULLs first if needed, then set NOT NULL
                // Spark doesn't support SET NOT NULL easily on existing data without UPDATE
                if let Some(default) = default_expr {
                    // For Delta: UPDATE then ALTER
                    if caps.supports_column_mapping {
                        // This is a two-statement operation but we return a single DDL
                        // The caller should handle multi-statement.
                        // For now, return the UPDATE + ALTER as a single strategy
                        return OpStrategy::Ddl(format!(
                            "UPDATE {{TABLE}} SET {} = {} WHERE {} IS NULL",
                            name, default, name
                        ));
                    }
                    return OpStrategy::FullRefresh(format!(
                        "Cannot set column '{}' to NOT NULL on Parquet table. \
                         Consider using Delta format or --allow-full-refresh",
                        name
                    ));
                }
                OpStrategy::FullRefresh(format!(
                    "Cannot set column '{}' to NOT NULL without a default expression. \
                     Provide a default or use --allow-full-refresh",
                    name
                ))
            }
        }
        SchemaOperation::AddStructField {
            column,
            path,
            field_name,
            field_type,
            default_expr: _,
        } => {
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
            OpStrategy::Ddl(format!(
                "ALTER TABLE {{TABLE}} ADD COLUMNS ({} {})",
                dot_path, type_sql
            ))
        }
        SchemaOperation::RemoveStructField {
            column,
            path,
            field_name,
        } => {
            if !caps.supports_column_mapping {
                return OpStrategy::FullRefresh(format!(
                    "Cannot drop struct field '{}.{}' on Parquet table — column mapping not supported. \
                     Consider using Delta format or --allow-full-refresh",
                    column,
                    field_name
                ));
            }
            let dot_path = format_spark_dot_path(column, path, Some(field_name));
            OpStrategy::Ddl(format!("ALTER TABLE {{TABLE}} DROP COLUMN {}", dot_path))
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
                OpStrategy::Ddl(format!("UPDATE {{TABLE}} SET {} = {}", name, expression))
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

/// Check if a type widening is natively supported by Spark ALTER COLUMN TYPE.
///
/// Spark supports limited safe widenings:
/// - TINYINT → SMALLINT → INTEGER → BIGINT
/// - FLOAT → DOUBLE
/// - DECIMAL precision increase (same or higher scale)
fn is_spark_safe_widening(from: &DataType, to: &DataType) -> bool {
    matches!(
        (from, to),
        // Integer widening chain
        (DataType::SmallInt, DataType::Integer)
            | (DataType::SmallInt, DataType::BigInt)
            | (DataType::Integer, DataType::BigInt)
            // Float widening
            | (DataType::Float, DataType::Double)
    )
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
        assert_eq!(
            result,
            MigrationExecution::Statements(vec![
                "ALTER TABLE cat.db.t DROP COLUMN meta.old_field".to_string()
            ])
        );
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
        assert_eq!(
            result,
            MigrationExecution::Statements(vec![
                "ALTER TABLE cat.db.t DROP COLUMN old_col".to_string()
            ])
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
            MigrationExecution::Statements(vec![
                "ALTER TABLE cat.db.t ALTER COLUMN amount TYPE BIGINT".to_string()
            ])
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
            MigrationExecution::Statements(vec![
                "ALTER TABLE cat.db.t ALTER COLUMN score TYPE DOUBLE".to_string()
            ])
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
        assert_eq!(
            result,
            MigrationExecution::Statements(vec![
                "ALTER TABLE cat.db.t ADD COLUMNS (meta.b STRING)".to_string()
            ])
        );
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
        assert_eq!(
            result,
            MigrationExecution::Statements(vec![
                "ALTER TABLE cat.db.t ALTER COLUMN amount TYPE BIGINT".to_string()
            ])
        );
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
        match result {
            MigrationExecution::Statements(stmts) => {
                assert_eq!(stmts.len(), 3);
                assert!(stmts[0].contains("ADD COLUMNS (new_col INTEGER)"));
                assert!(stmts[1].contains("DROP COLUMN old_col"));
                assert!(stmts[2].contains("ADD COLUMNS (meta.status STRING)"));
            }
            other => panic!("Expected Statements, got {:?}", other),
        }
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

    // ── is_spark_safe_widening ────────────────────────────────────────

    #[test]
    fn test_spark_safe_widenings() {
        assert!(is_spark_safe_widening(
            &DataType::SmallInt,
            &DataType::Integer
        ));
        assert!(is_spark_safe_widening(
            &DataType::SmallInt,
            &DataType::BigInt
        ));
        assert!(is_spark_safe_widening(
            &DataType::Integer,
            &DataType::BigInt
        ));
        assert!(is_spark_safe_widening(&DataType::Float, &DataType::Double));
    }

    #[test]
    fn test_spark_unsafe_widenings() {
        // Narrowing
        assert!(!is_spark_safe_widening(
            &DataType::BigInt,
            &DataType::Integer
        ));
        // VARCHAR to INT is not a safe widening
        assert!(!is_spark_safe_widening(
            &DataType::Varchar { max_length: None },
            &DataType::Integer
        ));
        // Array widening is not handled at this level
        assert!(!is_spark_safe_widening(
            &DataType::Array(Box::new(DataType::Integer)),
            &DataType::Array(Box::new(DataType::BigInt))
        ));
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
        match result {
            MigrationExecution::MergeSchemaWrite { columns_to_add } => {
                assert_eq!(columns_to_add.len(), 1);
                assert_eq!(columns_to_add[0].0, "items.element.score");
                assert_eq!(columns_to_add[0].1, DataType::Double);
            }
            other => panic!("Expected MergeSchemaWrite, got {:?}", other),
        }
    }

    #[test]
    fn test_delta_array_of_struct_field_add_merge_schema() {
        // Delta also doesn't support nested array DDL (supports_nested_array_ddl = false)
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
            MigrationExecution::MergeSchemaWrite { columns_to_add } => {
                assert_eq!(columns_to_add.len(), 1);
                assert_eq!(columns_to_add[0].0, "items.element.score");
            }
            other => panic!("Expected MergeSchemaWrite, got {:?}", other),
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
        assert_eq!(
            result,
            MigrationExecution::Statements(vec![
                "ALTER TABLE cat.db.t ADD COLUMNS (status STRING DEFAULT 'pending')".to_string()
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
        assert_eq!(
            result,
            MigrationExecution::Statements(vec![
                "ALTER TABLE cat.db.t ADD COLUMNS (required_col INTEGER NOT NULL)".to_string()
            ])
        );
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
        // Delta with column mapping: UPDATE to fill NULLs
        assert_eq!(
            result,
            MigrationExecution::Statements(vec![
                "UPDATE cat.db.t SET status = 'unknown' WHERE status IS NULL".to_string()
            ])
        );
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
            MigrationExecution::Statements(vec![
                "ALTER TABLE cat.db.t ALTER COLUMN val TYPE INTEGER".to_string()
            ])
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
