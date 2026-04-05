//! DuckDB-specific DDL generation from abstract `SchemaOperation`s.
//!
//! Translates backend-agnostic schema operations into DuckDB SQL statements.
//! DuckDB supports:
//! - Dot-notation for struct field access (`ALTER TABLE t ADD COLUMN col.field TYPE`)
//! - `ALTER COLUMN TYPE ... USING expr` for type rewrites (e.g., `struct_pack(...)`)
//! - `list_transform` for array element transformations

use crate::schema_tracking::{quote_identifier, SchemaOperation};
use smelt_types::DataType;

/// Generate DuckDB-specific DDL statements from a list of `SchemaOperation`s.
///
/// Returns a list of SQL statements to execute in order.
pub fn generate_duckdb_ddl(schema: &str, table: &str, ops: &[SchemaOperation]) -> Vec<String> {
    let qualified = format!("{}.{}", schema, table);
    let mut stmts = Vec::new();

    for op in ops {
        match op {
            SchemaOperation::AddColumn {
                name,
                data_type,
                nullable,
                default_expr,
            } => {
                let type_sql = data_type.to_sql();
                let qname = quote_identifier(name);
                let mut stmt = if *nullable {
                    format!(
                        "ALTER TABLE {} ADD COLUMN {} {}",
                        qualified, qname, type_sql
                    )
                } else {
                    format!(
                        "ALTER TABLE {} ADD COLUMN {} {} NOT NULL",
                        qualified, qname, type_sql
                    )
                };
                if let Some(default) = default_expr {
                    stmt.push_str(&format!(" DEFAULT {}", default));
                }
                stmts.push(stmt);
            }
            SchemaOperation::RemoveColumn { name } => {
                stmts.push(format!(
                    "ALTER TABLE {} DROP COLUMN {}",
                    qualified,
                    quote_identifier(name)
                ));
            }
            SchemaOperation::WidenColumnType { name, to, .. } => {
                stmts.push(format!(
                    "ALTER TABLE {} ALTER COLUMN {} TYPE {}",
                    qualified,
                    quote_identifier(name),
                    to.to_sql()
                ));
            }
            SchemaOperation::ChangeNullability {
                name,
                to_nullable,
                default_expr,
            } => {
                let qname = quote_identifier(name);
                if *to_nullable {
                    stmts.push(format!(
                        "ALTER TABLE {} ALTER COLUMN {} DROP NOT NULL",
                        qualified, qname
                    ));
                } else {
                    // Fill NULLs first, then set NOT NULL
                    if let Some(default) = default_expr {
                        stmts.push(format!(
                            "UPDATE {} SET {} = {} WHERE {} IS NULL",
                            qualified, qname, default, qname
                        ));
                    }
                    stmts.push(format!(
                        "ALTER TABLE {} ALTER COLUMN {} SET NOT NULL",
                        qualified, qname
                    ));
                }
            }
            SchemaOperation::AddStructField {
                column,
                path,
                field_name,
                field_type,
                default_expr: _,
            } => {
                let dot_path = format_dot_path(column, path, Some(field_name));
                stmts.push(format!(
                    "ALTER TABLE {} ADD COLUMN {} {}",
                    qualified,
                    dot_path,
                    field_type.to_sql()
                ));
            }
            SchemaOperation::RemoveStructField {
                column,
                path,
                field_name,
            } => {
                let dot_path = format_dot_path(column, path, Some(field_name));
                stmts.push(format!(
                    "ALTER TABLE {} DROP COLUMN {}",
                    qualified, dot_path
                ));
            }
            SchemaOperation::WidenNestedType {
                column,
                path,
                from: _,
                to,
            } => {
                // For DuckDB, nested type widening can use dot-notation ALTER COLUMN TYPE
                // when the path identifies a specific struct field.
                let qcol = quote_identifier(column);
                if path.is_empty() {
                    // Direct column type change — shouldn't normally hit this path
                    stmts.push(format!(
                        "ALTER TABLE {} ALTER COLUMN {} TYPE {}",
                        qualified,
                        qcol,
                        to.to_sql()
                    ));
                } else if path.len() == 1 && path[0] == "value" {
                    // Map value widening — reconstruct the full MAP type.
                    // The key type is unknown here, so use ALTER COLUMN TYPE on the whole column.
                    // DuckDB supports ALTER COLUMN TYPE for map columns.
                    let new_map_type = DataType::Map(
                        Box::new(DataType::Varchar { max_length: None }),
                        Box::new(to.clone()),
                    );
                    stmts.push(format!(
                        "ALTER TABLE {} ALTER COLUMN {} TYPE {}",
                        qualified,
                        qcol,
                        new_map_type.to_sql()
                    ));
                } else {
                    // Struct field widening — use dot-notation: ALTER COLUMN col.path.field TYPE new_type
                    let dot_path = format_dot_path(column, path, None);
                    stmts.push(format!(
                        "ALTER TABLE {} ALTER COLUMN {} TYPE {}",
                        qualified,
                        dot_path,
                        to.to_sql()
                    ));
                }
            }
            SchemaOperation::BackfillColumn { name, expression } => {
                stmts.push(format!(
                    "UPDATE {} SET {} = {}",
                    qualified,
                    quote_identifier(name),
                    expression
                ));
            }
            SchemaOperation::RewriteColumn {
                column,
                target_type,
                using_expr,
            } => {
                stmts.push(format!(
                    "ALTER TABLE {} ALTER COLUMN {} TYPE {} USING {}",
                    qualified,
                    quote_identifier(column),
                    target_type.to_sql(),
                    using_expr
                ));
            }
        }
    }

    stmts
}

/// Build a `struct_pack(...)` expression that transforms a struct column from
/// `old_type` to `new_type`.
///
/// Handles:
/// - New fields (added with `NULL::TYPE` or default expression)
/// - Widened fields (cast via `::NEW_TYPE`)
/// - Unchanged fields (passed through as-is)
/// - Nested structs (recursive `struct_pack`)
pub fn build_struct_pack_expr(
    column: &str,
    old_type: &DataType,
    new_type: &DataType,
) -> Option<String> {
    match (old_type, new_type) {
        (DataType::Struct(old_fields), DataType::Struct(new_fields)) => {
            let parts = build_struct_pack_parts(column, old_fields, new_fields);
            Some(format!("struct_pack({})", parts.join(", ")))
        }
        _ => None,
    }
}

/// Build the individual field assignments for a struct_pack expression.
fn build_struct_pack_parts(
    column: &str,
    old_fields: &[(String, DataType)],
    new_fields: &[(String, DataType)],
) -> Vec<String> {
    let mut parts = Vec::new();

    for (new_name, new_dt) in new_fields {
        if let Some((_old_name, old_dt)) = old_fields.iter().find(|(n, _)| n == new_name) {
            // Field exists in old type
            let field_ref = format!("{}.{}", column, quote_identifier(new_name));
            if old_dt == new_dt {
                // Unchanged — pass through
                parts.push(format!("{} := {}", new_name, field_ref));
            } else {
                // Type changed — check if it's a nested struct needing recursive struct_pack
                match (old_dt, new_dt) {
                    (DataType::Struct(old_inner), DataType::Struct(new_inner)) => {
                        let inner_expr = build_struct_pack_inner(&field_ref, old_inner, new_inner);
                        parts.push(format!("{} := {}", new_name, inner_expr));
                    }
                    _ => {
                        // Simple type cast
                        parts.push(format!(
                            "{} := {}::{}",
                            new_name,
                            field_ref,
                            new_dt.to_sql()
                        ));
                    }
                }
            }
        } else {
            // New field — use NULL cast to correct type
            parts.push(format!("{} := NULL::{}", new_name, new_dt.to_sql()));
        }
    }

    parts
}

/// Recursively build a nested struct_pack expression for inner structs.
fn build_struct_pack_inner(
    field_ref: &str,
    old_fields: &[(String, DataType)],
    new_fields: &[(String, DataType)],
) -> String {
    let parts = build_struct_pack_parts(field_ref, old_fields, new_fields);
    format!("struct_pack({})", parts.join(", "))
}

/// Build a `list_transform(column, x -> struct_pack(...))` expression for
/// array-of-struct widening in DuckDB.
///
/// Returns `None` if either type is not `Array(Struct(...))`.
pub fn build_list_transform_expr(
    column: &str,
    old_type: &DataType,
    new_type: &DataType,
) -> Option<String> {
    match (old_type, new_type) {
        (DataType::Array(old_elem), DataType::Array(new_elem)) => {
            match (old_elem.as_ref(), new_elem.as_ref()) {
                (DataType::Struct(old_fields), DataType::Struct(new_fields)) => {
                    let parts = build_struct_pack_parts("x", old_fields, new_fields);
                    Some(format!(
                        "list_transform({}, x -> struct_pack({}))",
                        column,
                        parts.join(", ")
                    ))
                }
                _ => {
                    // Simple array element cast — no list_transform needed,
                    // DuckDB handles this with ALTER COLUMN TYPE directly
                    None
                }
            }
        }
        _ => None,
    }
}

/// Build a USING expression for column type changes.
///
/// Determines whether to use `struct_pack`, `list_transform`, or a simple cast
/// based on the old and new types.
pub fn build_using_expr(column: &str, old_type: &DataType, new_type: &DataType) -> Option<String> {
    // Try struct_pack first (struct columns)
    if let Some(expr) = build_struct_pack_expr(column, old_type, new_type) {
        return Some(expr);
    }
    // Try list_transform (array-of-struct columns)
    if let Some(expr) = build_list_transform_expr(column, old_type, new_type) {
        return Some(expr);
    }
    None
}

/// Format a dot-separated path for DuckDB struct field access.
/// e.g., `format_dot_path("meta", &["inner"], Some("field"))` → `"meta.inner.field"`
fn format_dot_path(column: &str, path: &[String], leaf: Option<&str>) -> String {
    let mut parts = vec![quote_identifier(column)];
    parts.extend(path.iter().map(|p| quote_identifier(p)));
    if let Some(l) = leaf {
        parts.push(quote_identifier(l));
    }
    parts.join(".")
}

#[cfg(test)]
mod tests {
    use super::*;
    use smelt_types::DataType;

    // ── generate_duckdb_ddl tests ──────────────────────────────────────

    #[test]
    fn test_add_column_nullable() {
        let ops = vec![SchemaOperation::AddColumn {
            name: "status".into(),
            data_type: DataType::Varchar { max_length: None },
            nullable: true,
            default_expr: None,
        }];
        let ddl = generate_duckdb_ddl("main", "t", &ops);
        assert_eq!(ddl, vec!["ALTER TABLE main.t ADD COLUMN status VARCHAR"]);
    }

    #[test]
    fn test_add_column_not_null_with_default() {
        let ops = vec![SchemaOperation::AddColumn {
            name: "count".into(),
            data_type: DataType::Integer,
            nullable: false,
            default_expr: Some("0".into()),
        }];
        let ddl = generate_duckdb_ddl("main", "t", &ops);
        assert_eq!(
            ddl,
            vec!["ALTER TABLE main.t ADD COLUMN count INTEGER NOT NULL DEFAULT 0"]
        );
    }

    #[test]
    fn test_remove_column() {
        let ops = vec![SchemaOperation::RemoveColumn {
            name: "old_col".into(),
        }];
        let ddl = generate_duckdb_ddl("main", "t", &ops);
        assert_eq!(ddl, vec!["ALTER TABLE main.t DROP COLUMN old_col"]);
    }

    #[test]
    fn test_widen_column_type() {
        let ops = vec![SchemaOperation::WidenColumnType {
            name: "amount".into(),
            from: DataType::Integer,
            to: DataType::BigInt,
        }];
        let ddl = generate_duckdb_ddl("main", "t", &ops);
        assert_eq!(
            ddl,
            vec!["ALTER TABLE main.t ALTER COLUMN amount TYPE BIGINT"]
        );
    }

    #[test]
    fn test_change_nullability_to_nullable() {
        let ops = vec![SchemaOperation::ChangeNullability {
            name: "status".into(),
            to_nullable: true,
            default_expr: None,
        }];
        let ddl = generate_duckdb_ddl("main", "t", &ops);
        assert_eq!(
            ddl,
            vec!["ALTER TABLE main.t ALTER COLUMN status DROP NOT NULL"]
        );
    }

    #[test]
    fn test_change_nullability_to_not_null() {
        let ops = vec![SchemaOperation::ChangeNullability {
            name: "status".into(),
            to_nullable: false,
            default_expr: Some("'unknown'".into()),
        }];
        let ddl = generate_duckdb_ddl("main", "t", &ops);
        assert_eq!(
            ddl,
            vec![
                "UPDATE main.t SET status = 'unknown' WHERE status IS NULL",
                "ALTER TABLE main.t ALTER COLUMN status SET NOT NULL",
            ]
        );
    }

    #[test]
    fn test_add_struct_field() {
        let ops = vec![SchemaOperation::AddStructField {
            column: "meta".into(),
            path: vec![],
            field_name: "b".into(),
            field_type: DataType::Varchar { max_length: None },
            default_expr: None,
        }];
        let ddl = generate_duckdb_ddl("main", "t", &ops);
        assert_eq!(ddl, vec!["ALTER TABLE main.t ADD COLUMN meta.b VARCHAR"]);
    }

    #[test]
    fn test_add_nested_struct_field() {
        let ops = vec![SchemaOperation::AddStructField {
            column: "data".into(),
            path: vec!["inner".into()],
            field_name: "y".into(),
            field_type: DataType::Varchar { max_length: None },
            default_expr: None,
        }];
        let ddl = generate_duckdb_ddl("main", "t", &ops);
        assert_eq!(
            ddl,
            vec!["ALTER TABLE main.t ADD COLUMN data.\"inner\".y VARCHAR"]
        );
    }

    #[test]
    fn test_remove_struct_field() {
        let ops = vec![SchemaOperation::RemoveStructField {
            column: "meta".into(),
            path: vec![],
            field_name: "old_field".into(),
        }];
        let ddl = generate_duckdb_ddl("main", "t", &ops);
        assert_eq!(ddl, vec!["ALTER TABLE main.t DROP COLUMN meta.old_field"]);
    }

    #[test]
    fn test_rewrite_column_struct_pack() {
        let ops = vec![SchemaOperation::RewriteColumn {
            column: "meta".into(),
            target_type: DataType::Struct(vec![
                ("a".into(), DataType::BigInt),
                ("b".into(), DataType::Varchar { max_length: None }),
            ]),
            using_expr: "struct_pack(a := meta.a::BIGINT, b := meta.b)".into(),
        }];
        let ddl = generate_duckdb_ddl("main", "t", &ops);
        assert_eq!(
            ddl,
            vec![
                "ALTER TABLE main.t ALTER COLUMN meta TYPE STRUCT(a BIGINT, b VARCHAR) USING struct_pack(a := meta.a::BIGINT, b := meta.b)"
            ]
        );
    }

    #[test]
    fn test_widen_array_column() {
        let ops = vec![SchemaOperation::WidenColumnType {
            name: "scores".into(),
            from: DataType::Array(Box::new(DataType::Integer)),
            to: DataType::Array(Box::new(DataType::BigInt)),
        }];
        let ddl = generate_duckdb_ddl("main", "t", &ops);
        assert_eq!(
            ddl,
            vec!["ALTER TABLE main.t ALTER COLUMN scores TYPE BIGINT[]"]
        );
    }

    #[test]
    fn test_backfill_column() {
        let ops = vec![SchemaOperation::BackfillColumn {
            name: "status".into(),
            expression: "CASE WHEN active THEN 'active' ELSE 'inactive' END".into(),
        }];
        let ddl = generate_duckdb_ddl("main", "t", &ops);
        assert_eq!(
            ddl,
            vec!["UPDATE main.t SET status = CASE WHEN active THEN 'active' ELSE 'inactive' END"]
        );
    }

    #[test]
    fn test_multiple_operations() {
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
            SchemaOperation::WidenColumnType {
                name: "amount".into(),
                from: DataType::Integer,
                to: DataType::BigInt,
            },
        ];
        let ddl = generate_duckdb_ddl("main", "t", &ops);
        assert_eq!(ddl.len(), 3);
        assert_eq!(ddl[0], "ALTER TABLE main.t ADD COLUMN new_col INTEGER");
        assert_eq!(ddl[1], "ALTER TABLE main.t DROP COLUMN old_col");
        assert_eq!(ddl[2], "ALTER TABLE main.t ALTER COLUMN amount TYPE BIGINT");
    }

    // ── build_struct_pack_expr tests ───────────────────────────────────

    #[test]
    fn test_struct_pack_field_widening() {
        let old = DataType::Struct(vec![
            ("a".into(), DataType::Integer),
            ("b".into(), DataType::Varchar { max_length: None }),
        ]);
        let new = DataType::Struct(vec![
            ("a".into(), DataType::BigInt),
            ("b".into(), DataType::Varchar { max_length: None }),
        ]);
        let expr = build_struct_pack_expr("meta", &old, &new).unwrap();
        assert_eq!(expr, "struct_pack(a := meta.a::BIGINT, b := meta.b)");
    }

    #[test]
    fn test_struct_pack_new_field() {
        let old = DataType::Struct(vec![
            ("a".into(), DataType::Integer),
            ("b".into(), DataType::Varchar { max_length: None }),
        ]);
        let new = DataType::Struct(vec![
            ("a".into(), DataType::Integer),
            ("b".into(), DataType::Varchar { max_length: None }),
            ("c".into(), DataType::Boolean),
        ]);
        let expr = build_struct_pack_expr("meta", &old, &new).unwrap();
        assert_eq!(
            expr,
            "struct_pack(a := meta.a, b := meta.b, c := NULL::BOOLEAN)"
        );
    }

    #[test]
    fn test_struct_pack_widen_and_add() {
        let old = DataType::Struct(vec![
            ("a".into(), DataType::Integer),
            ("b".into(), DataType::Varchar { max_length: None }),
        ]);
        let new = DataType::Struct(vec![
            ("a".into(), DataType::BigInt),
            ("b".into(), DataType::Varchar { max_length: None }),
            ("c".into(), DataType::Boolean),
        ]);
        let expr = build_struct_pack_expr("meta", &old, &new).unwrap();
        assert_eq!(
            expr,
            "struct_pack(a := meta.a::BIGINT, b := meta.b, c := NULL::BOOLEAN)"
        );
    }

    #[test]
    fn test_struct_pack_nested_struct() {
        let old = DataType::Struct(vec![
            (
                "inner".into(),
                DataType::Struct(vec![("x".into(), DataType::Integer)]),
            ),
            ("b".into(), DataType::BigInt),
        ]);
        let new = DataType::Struct(vec![
            (
                "inner".into(),
                DataType::Struct(vec![
                    ("x".into(), DataType::BigInt),
                    ("y".into(), DataType::Varchar { max_length: None }),
                ]),
            ),
            ("b".into(), DataType::BigInt),
        ]);
        let expr = build_struct_pack_expr("data", &old, &new).unwrap();
        assert_eq!(
            expr,
            "struct_pack(inner := struct_pack(x := data.\"inner\".x::BIGINT, y := NULL::VARCHAR), b := data.b)"
        );
    }

    #[test]
    fn test_struct_pack_non_struct_returns_none() {
        let old = DataType::Integer;
        let new = DataType::BigInt;
        assert!(build_struct_pack_expr("col", &old, &new).is_none());
    }

    #[test]
    fn test_widen_nested_type_struct_field() {
        let ops = vec![SchemaOperation::WidenNestedType {
            column: "meta".into(),
            path: vec!["a".into()],
            from: DataType::Integer,
            to: DataType::BigInt,
        }];
        let ddl = generate_duckdb_ddl("main", "t", &ops);
        assert_eq!(
            ddl,
            vec!["ALTER TABLE main.t ALTER COLUMN meta.a TYPE BIGINT"]
        );
    }

    #[test]
    fn test_widen_nested_type_map_value() {
        let ops = vec![SchemaOperation::WidenNestedType {
            column: "m".into(),
            path: vec!["value".into()],
            from: DataType::Integer,
            to: DataType::BigInt,
        }];
        let ddl = generate_duckdb_ddl("main", "t", &ops);
        assert_eq!(
            ddl,
            vec!["ALTER TABLE main.t ALTER COLUMN m TYPE MAP(VARCHAR, BIGINT)"]
        );
    }

    #[test]
    fn test_widen_nested_type_deeply_nested() {
        let ops = vec![SchemaOperation::WidenNestedType {
            column: "data".into(),
            path: vec!["inner".into(), "x".into()],
            from: DataType::Integer,
            to: DataType::BigInt,
        }];
        let ddl = generate_duckdb_ddl("main", "t", &ops);
        assert_eq!(
            ddl,
            vec!["ALTER TABLE main.t ALTER COLUMN data.\"inner\".x TYPE BIGINT"]
        );
    }

    // ── build_list_transform_expr tests ────────────────────────────────

    #[test]
    fn test_list_transform_array_of_struct_widen() {
        let old = DataType::Array(Box::new(DataType::Struct(vec![(
            "a".into(),
            DataType::Integer,
        )])));
        let new = DataType::Array(Box::new(DataType::Struct(vec![
            ("a".into(), DataType::BigInt),
            ("b".into(), DataType::Varchar { max_length: None }),
        ])));
        let expr = build_list_transform_expr("items", &old, &new).unwrap();
        assert_eq!(
            expr,
            "list_transform(items, x -> struct_pack(a := x.a::BIGINT, b := NULL::VARCHAR))"
        );
    }

    #[test]
    fn test_list_transform_simple_array_returns_none() {
        let old = DataType::Array(Box::new(DataType::Integer));
        let new = DataType::Array(Box::new(DataType::BigInt));
        assert!(build_list_transform_expr("scores", &old, &new).is_none());
    }

    #[test]
    fn test_list_transform_non_array_returns_none() {
        let old = DataType::Integer;
        let new = DataType::BigInt;
        assert!(build_list_transform_expr("col", &old, &new).is_none());
    }

    // ── build_using_expr tests ─────────────────────────────────────────

    #[test]
    fn test_using_expr_struct() {
        let old = DataType::Struct(vec![("a".into(), DataType::Integer)]);
        let new = DataType::Struct(vec![("a".into(), DataType::BigInt)]);
        let expr = build_using_expr("meta", &old, &new).unwrap();
        assert!(expr.starts_with("struct_pack("));
    }

    #[test]
    fn test_using_expr_array_of_struct() {
        let old = DataType::Array(Box::new(DataType::Struct(vec![(
            "a".into(),
            DataType::Integer,
        )])));
        let new = DataType::Array(Box::new(DataType::Struct(vec![(
            "a".into(),
            DataType::BigInt,
        )])));
        let expr = build_using_expr("items", &old, &new).unwrap();
        assert!(expr.starts_with("list_transform("));
    }

    #[test]
    fn test_using_expr_simple_types_returns_none() {
        assert!(build_using_expr("col", &DataType::Integer, &DataType::BigInt).is_none());
    }

    // ── struct_pack additional tests ───────────────────────────────────

    #[test]
    fn test_struct_pack_all_unchanged() {
        let dt = DataType::Struct(vec![
            ("a".into(), DataType::Integer),
            ("b".into(), DataType::Varchar { max_length: None }),
        ]);
        let expr = build_struct_pack_expr("meta", &dt, &dt).unwrap();
        assert_eq!(expr, "struct_pack(a := meta.a, b := meta.b)");
    }

    // ── quoting tests ────────────────────────────────────────────────────

    #[test]
    fn test_add_column_with_keyword_name() {
        let ops = vec![SchemaOperation::AddColumn {
            name: "select".to_string(),
            data_type: DataType::Integer,
            nullable: true,
            default_expr: None,
        }];
        let stmts = generate_duckdb_ddl("main", "t", &ops);
        assert_eq!(stmts.len(), 1);
        assert!(
            stmts[0].contains("\"select\""),
            "SQL keyword column name should be quoted, got: {}",
            stmts[0]
        );
    }

    #[test]
    fn test_add_struct_field_with_space_in_name() {
        let ops = vec![SchemaOperation::AddStructField {
            column: "meta".to_string(),
            path: vec![],
            field_name: "first name".to_string(),
            field_type: DataType::Varchar { max_length: None },
            default_expr: None,
        }];
        let stmts = generate_duckdb_ddl("main", "t", &ops);
        assert_eq!(stmts.len(), 1);
        assert!(
            stmts[0].contains("\"first name\""),
            "Field name with space should be quoted, got: {}",
            stmts[0]
        );
    }

    #[test]
    fn test_remove_column_with_keyword_name() {
        let ops = vec![SchemaOperation::RemoveColumn {
            name: "order".to_string(),
        }];
        let stmts = generate_duckdb_ddl("main", "t", &ops);
        assert_eq!(stmts.len(), 1);
        assert!(
            stmts[0].contains("\"order\""),
            "SQL keyword column name should be quoted, got: {}",
            stmts[0]
        );
    }
}
