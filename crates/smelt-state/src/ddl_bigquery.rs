//! GoogleSQL (BigQuery) DDL generation from abstract `SchemaOperation`s.
//!
//! Every rule below is a *measured* GoogleSQL fact, established against a live
//! warehouse by `scripts/bigquery-probe-ddl.sh`. GoogleSQL differs from the
//! DuckDB generator's SQL in four ways that each make the DuckDB statement a
//! hard error rather than a dialect wobble:
//!
//! - **Type names.** `VARCHAR`, `TEXT`, `DOUBLE`, `FLOAT`, `REAL`, `CHAR(n)`
//!   and `BLOB` are each `Type not found`. `STRUCT(a INT64)` and `INT64[]` are
//!   syntax errors — GoogleSQL spells them `STRUCT<a INT64>` and
//!   `ARRAY<INT64>` — and there is no `MAP` type at all.
//! - **Widening.** The spelling is `ALTER COLUMN c SET DATA TYPE t`;
//!   DuckDB's `ALTER COLUMN c TYPE t` is a syntax error, and there is no
//!   `USING` clause to rewrite a value with.
//! - **Constraints.** A column cannot be added `NOT NULL`
//!   (`Cannot add required fields to an existing schema`) nor with a `DEFAULT`
//!   in the same statement, and there is no `ALTER COLUMN … SET NOT NULL` at
//!   all. Only the relaxing direction, `DROP NOT NULL`, exists.
//! - **Nesting.** There is no dotted `ADD COLUMN s.b` / `DROP COLUMN s.a`, and
//!   `SET DATA TYPE` demands the old type be *assignable* to the new one —
//!   which a struct that gained or lost a field is not, and
//!   `ARRAY<INT64> → ARRAY<NUMERIC>` is not either.
//!
//! What GoogleSQL cannot express resolves to a full refresh carrying a reason
//! that names the column and the limitation. That refusal is the whole point:
//! the alternative is DDL the warehouse rejects mid-run.

use crate::schema_tracking::{DeployedColumn, SchemaOperation};
use smelt_types::DataType;

/// Result of planning a migration for BigQuery.
///
/// BigQuery has no counterpart to Spark's `TableRewrite` or `MergeSchemaWrite`
/// strategies — either GoogleSQL can express the change as DDL, or the model
/// is rebuilt from source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BigQueryMigration {
    /// DDL statements to execute in order.
    Statements(Vec<String>),
    /// GoogleSQL cannot express this change — needs `--allow-full-refresh`.
    FullRefreshRequired { reason: String },
}

/// Quote one identifier for GoogleSQL.
///
/// GoogleSQL quotes with backticks; the double quotes `quote_identifier`
/// emits are a *string literal* there, not an identifier
/// (`Unexpected string literal "c"` — measured).
fn quote_ident(ident: &str) -> String {
    format!("`{}`", ident.replace('`', "\\`"))
}

/// Quote a `schema.table` path as one backticked path.
///
/// Backticking the whole path rather than each part keeps a schema that
/// already carries a project prefix (`project.dataset`) working, which is the
/// same shape `smelt_backend_bigquery::sql::qualified_name` produces.
fn qualified(schema: &str, table: &str) -> String {
    format!("`{}.{}`", schema, table)
}

/// Render a `DataType` as a GoogleSQL type name.
///
/// `Err` carries the reason the type has no GoogleSQL spelling, for a refusal
/// message — it is never a silent fallback.
pub fn bigquery_type_sql(dt: &DataType) -> Result<String, String> {
    Ok(match dt {
        DataType::Boolean => "BOOL".to_string(),
        // GoogleSQL has exactly one integer type; the narrower names are
        // accepted aliases for it, so the canonical spelling is emitted.
        DataType::SmallInt | DataType::Integer | DataType::BigInt => "INT64".to_string(),
        DataType::Float | DataType::Double => "FLOAT64".to_string(),
        DataType::Decimal { precision, scale } => {
            // NUMERIC caps at 9 fractional and 29 integer digits; wider
            // decimals are BIGNUMERIC (38 and 38). Beyond that GoogleSQL has
            // no exact-decimal type, and silently reaching for FLOAT64 would
            // trade an error for lost precision.
            let (p, s) = (i64::from(*precision), i64::from(*scale));
            if s <= 9 && p - s <= 29 {
                format!("NUMERIC({},{})", p, s)
            } else if s <= 38 && p - s <= 38 {
                format!("BIGNUMERIC({},{})", p, s)
            } else {
                return Err(format!(
                    "DECIMAL({},{}) exceeds BIGNUMERIC's 38 integer and 38 fractional digits",
                    p, s
                ));
            }
        }
        // GoogleSQL's one string type is unparameterised-or-`STRING(n)`;
        // `VARCHAR`, `TEXT` and `CHAR` are each `Type not found`. The length
        // is dropped rather than carried over: a bound is a DuckDB-side
        // constraint, and STRING accepts every value STRING(n) would.
        DataType::Text | DataType::Varchar { .. } | DataType::Char { .. } => "STRING".to_string(),
        DataType::Blob => "BYTES".to_string(),
        DataType::Date => "DATE".to_string(),
        DataType::Time => "TIME".to_string(),
        // GoogleSQL's TIMESTAMP is the instant type and DATETIME the naive
        // one, but the rest of the BigQuery backend prints both smelt
        // timestamps as TIMESTAMP (`smelt_dialect::type_conformance`), and a
        // DDL type that disagreed with the cast wrap would migrate a column
        // the next write could not fill.
        DataType::Timestamp { .. } => "TIMESTAMP".to_string(),
        DataType::Interval => "INTERVAL".to_string(),
        DataType::Array(inner) => format!("ARRAY<{}>", bigquery_type_sql(inner)?),
        DataType::Struct(fields) => {
            let rendered: Result<Vec<String>, String> = fields
                .iter()
                .map(|(name, ty)| Ok(format!("{} {}", name, bigquery_type_sql(ty)?)))
                .collect();
            format!("STRUCT<{}>", rendered?.join(", "))
        }
        DataType::Map(_, _) => return Err("GoogleSQL has no MAP type".to_string()),
        DataType::Null => return Err("GoogleSQL has no NULL column type".to_string()),
        DataType::Unknown(reason) => {
            return Err(format!("type could not be inferred ({:?})", reason))
        }
    })
}

/// Generate GoogleSQL migration statements from a list of `SchemaOperation`s.
///
/// # Arguments
/// * `schema` — dataset (optionally project-qualified, `project.dataset`)
/// * `table` — table name
/// * `ops` — abstract schema operations to execute
/// * `deployed` — the live table's columns, consulted for the one case where
///   GoogleSQL's answer depends on the *existing* column rather than the
///   operation: `SET DATA TYPE` on a `REQUIRED` column is refused
///   (`Required field c cannot be null`), so widening one is planned as a full
///   refresh rather than left to fail mid-run.
pub fn generate_bigquery_ddl(
    schema: &str,
    table: &str,
    ops: &[SchemaOperation],
    deployed: &[DeployedColumn],
) -> BigQueryMigration {
    let qualified = qualified(schema, table);
    let mut stmts = Vec::new();

    macro_rules! refuse {
        ($($arg:tt)*) => {
            return BigQueryMigration::FullRefreshRequired { reason: format!($($arg)*) }
        };
    }

    for op in ops {
        match op {
            SchemaOperation::AddColumn {
                name,
                data_type,
                nullable,
                default_expr,
            } => {
                if !*nullable {
                    refuse!(
                        "GoogleSQL cannot add the NOT NULL column '{}' to an existing table \
                         (`Cannot add required fields to an existing schema`)",
                        name
                    );
                }
                let type_sql = match bigquery_type_sql(data_type) {
                    Ok(t) => t,
                    Err(why) => refuse!("column '{}' has no GoogleSQL type: {}", name, why),
                };
                let qname = quote_ident(name);
                stmts.push(format!(
                    "ALTER TABLE {} ADD COLUMN {} {}",
                    qualified, qname, type_sql
                ));
                if let Some(default) = default_expr {
                    // A DEFAULT cannot ride along on the ADD (`Add field with
                    // default value to an existing table schema is not
                    // supported`), so it is set in a second statement — and a
                    // BigQuery default governs only *subsequent* inserts,
                    // whereas DuckDB's ADD COLUMN … DEFAULT also fills the
                    // rows already there. The UPDATE restores that half.
                    stmts.push(format!(
                        "ALTER TABLE {} ALTER COLUMN {} SET DEFAULT {}",
                        qualified, qname, default
                    ));
                    stmts.push(format!(
                        "UPDATE {} SET {} = {} WHERE {} IS NULL",
                        qualified, qname, default, qname
                    ));
                }
            }
            SchemaOperation::RemoveColumn { name } => {
                stmts.push(format!(
                    "ALTER TABLE {} DROP COLUMN {}",
                    qualified,
                    quote_ident(name)
                ));
            }
            SchemaOperation::WidenColumnType { name, to, .. } => {
                if let DataType::Array(_) = to {
                    refuse!(
                        "GoogleSQL cannot widen the element type of array column '{}' — \
                         ARRAY<T> is not assignable to ARRAY<wider T>",
                        name
                    );
                }
                let type_sql = match bigquery_type_sql(to) {
                    Ok(t) => t,
                    Err(why) => refuse!("column '{}' has no GoogleSQL type: {}", name, why),
                };
                if deployed.iter().any(|c| c.name == *name && !c.nullable) {
                    refuse!(
                        "GoogleSQL cannot widen the type of REQUIRED column '{}' \
                         (`Required field {} cannot be null`)",
                        name,
                        name
                    );
                }
                stmts.push(format!(
                    "ALTER TABLE {} ALTER COLUMN {} SET DATA TYPE {}",
                    qualified,
                    quote_ident(name),
                    type_sql
                ));
            }
            SchemaOperation::ChangeNullability {
                name,
                to_nullable,
                default_expr,
            } => {
                if !*to_nullable {
                    refuse!(
                        "GoogleSQL has no ALTER COLUMN … SET NOT NULL, so column '{}' \
                         cannot be tightened to NOT NULL in place",
                        name
                    );
                }
                let qname = quote_ident(name);
                // A relaxing change keeps the DuckDB generator's shape: the
                // fill expression, when one is declared, still applies.
                if let Some(default) = default_expr {
                    stmts.push(format!(
                        "UPDATE {} SET {} = {} WHERE {} IS NULL",
                        qualified, qname, default, qname
                    ));
                }
                stmts.push(format!(
                    "ALTER TABLE {} ALTER COLUMN {} DROP NOT NULL",
                    qualified, qname
                ));
            }
            SchemaOperation::AddStructField {
                column, field_name, ..
            } => {
                refuse!(
                    "GoogleSQL cannot add field '{}' to struct column '{}': it has no dotted \
                     ADD COLUMN, and SET DATA TYPE refuses a struct that gained a field",
                    field_name,
                    column
                );
            }
            SchemaOperation::RemoveStructField {
                column, field_name, ..
            } => {
                refuse!(
                    "GoogleSQL cannot drop field '{}' from struct column '{}': it has no dotted \
                     DROP COLUMN, and SET DATA TYPE refuses a struct that lost a field",
                    field_name,
                    column
                );
            }
            SchemaOperation::WidenNestedType { column, .. } => {
                refuse!(
                    "GoogleSQL cannot widen a type nested inside column '{}' — SET DATA TYPE \
                     takes the whole column type, and neither an array element nor a map value \
                     is assignable to a wider one",
                    column
                );
            }
            SchemaOperation::BackfillColumn { name, expression } => {
                // BigQuery requires a WHERE on every UPDATE; DuckDB's
                // generator emits none, so the always-true predicate stands in
                // for it and keeps the same all-rows scope.
                stmts.push(format!(
                    "UPDATE {} SET {} = {} WHERE TRUE",
                    qualified,
                    quote_ident(name),
                    expression
                ));
            }
            SchemaOperation::RewriteColumn { column, .. } => {
                refuse!(
                    "GoogleSQL has no ALTER COLUMN … USING, so column '{}' cannot be rewritten \
                     in place",
                    column
                );
            }
        }
    }

    BigQueryMigration::Statements(stmts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deployed(name: &str, nullable: bool) -> DeployedColumn {
        DeployedColumn {
            name: name.to_string(),
            data_type: "INT64".to_string(),
            nullable,
        }
    }

    fn stmts(ops: &[SchemaOperation]) -> Vec<String> {
        match generate_bigquery_ddl("ds", "t", ops, &[]) {
            BigQueryMigration::Statements(s) => s,
            other => panic!("expected statements, got {:?}", other),
        }
    }

    fn reason(ops: &[SchemaOperation]) -> String {
        match generate_bigquery_ddl("ds", "t", ops, &[]) {
            BigQueryMigration::FullRefreshRequired { reason } => reason,
            other => panic!("expected a refusal, got {:?}", other),
        }
    }

    #[test]
    fn type_names_are_the_googlesql_spellings() {
        let cases = [
            (DataType::Boolean, "BOOL"),
            (DataType::SmallInt, "INT64"),
            (DataType::Integer, "INT64"),
            (DataType::BigInt, "INT64"),
            (DataType::Float, "FLOAT64"),
            (DataType::Double, "FLOAT64"),
            (DataType::Text, "STRING"),
            (DataType::Varchar { max_length: None }, "STRING"),
            (
                DataType::Varchar {
                    max_length: Some(10),
                },
                "STRING",
            ),
            (DataType::Char { length: 3 }, "STRING"),
            (DataType::Blob, "BYTES"),
            (DataType::Date, "DATE"),
            (DataType::Time, "TIME"),
            (
                DataType::Timestamp {
                    with_timezone: false,
                },
                "TIMESTAMP",
            ),
            (
                DataType::Timestamp {
                    with_timezone: true,
                },
                "TIMESTAMP",
            ),
            (DataType::Interval, "INTERVAL"),
            (
                DataType::Decimal {
                    precision: 10,
                    scale: 2,
                },
                "NUMERIC(10,2)",
            ),
            (DataType::Array(Box::new(DataType::Integer)), "ARRAY<INT64>"),
            (
                DataType::Struct(vec![("a".to_string(), DataType::Text)]),
                "STRUCT<a STRING>",
            ),
        ];
        for (dt, expected) in cases {
            assert_eq!(
                bigquery_type_sql(&dt).unwrap(),
                expected,
                "wrong GoogleSQL spelling for {:?}",
                dt
            );
        }
    }

    #[test]
    fn wide_decimals_become_bignumeric_and_wider_still_are_refused() {
        // NUMERIC caps at 29 integer digits — measured
        // (`In NUMERIC(P, 2), P must be between 2 and 31`).
        assert_eq!(
            bigquery_type_sql(&DataType::Decimal {
                precision: 40,
                scale: 2
            })
            .unwrap(),
            "BIGNUMERIC(40,2)"
        );
        assert!(bigquery_type_sql(&DataType::Decimal {
            precision: 90,
            scale: 2
        })
        .is_err());
    }

    #[test]
    fn map_has_no_googlesql_type() {
        let err = bigquery_type_sql(&DataType::Map(
            Box::new(DataType::Text),
            Box::new(DataType::Integer),
        ))
        .unwrap_err();
        assert!(err.contains("MAP"), "the refusal must name the type: {err}");
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
            vec!["ALTER TABLE `ds.t` ADD COLUMN `amount` INT64"]
        );
    }

    #[test]
    fn add_column_with_default_sets_it_separately_and_fills_existing_rows() {
        assert_eq!(
            stmts(&[SchemaOperation::AddColumn {
                name: "amount".into(),
                data_type: DataType::BigInt,
                nullable: true,
                default_expr: Some("0".into()),
            }]),
            vec![
                "ALTER TABLE `ds.t` ADD COLUMN `amount` INT64",
                "ALTER TABLE `ds.t` ALTER COLUMN `amount` SET DEFAULT 0",
                "UPDATE `ds.t` SET `amount` = 0 WHERE `amount` IS NULL",
            ]
        );
    }

    #[test]
    fn required_column_add_is_refused_naming_the_column() {
        let why = reason(&[SchemaOperation::AddColumn {
            name: "amount".into(),
            data_type: DataType::BigInt,
            nullable: false,
            default_expr: Some("0".into()),
        }]);
        assert!(why.contains("amount") && why.contains("NOT NULL"), "{why}");
    }

    #[test]
    fn widen_uses_set_data_type() {
        assert_eq!(
            stmts(&[SchemaOperation::WidenColumnType {
                name: "amount".into(),
                from: DataType::Integer,
                to: DataType::Decimal {
                    precision: 10,
                    scale: 4
                },
            }]),
            vec!["ALTER TABLE `ds.t` ALTER COLUMN `amount` SET DATA TYPE NUMERIC(10,4)"]
        );
    }

    #[test]
    fn widening_a_required_column_is_refused() {
        // Measured: BigQuery answers `Required field c cannot be null`.
        let op = [SchemaOperation::WidenColumnType {
            name: "amount".into(),
            from: DataType::Integer,
            to: DataType::Double,
        }];
        match generate_bigquery_ddl("ds", "t", &op, &[deployed("amount", false)]) {
            BigQueryMigration::FullRefreshRequired { reason } => {
                assert!(reason.contains("amount"), "{reason}");
            }
            other => panic!("expected a refusal, got {:?}", other),
        }
        // The same widening on a NULLABLE column is DDL, not a refusal.
        match generate_bigquery_ddl("ds", "t", &op, &[deployed("amount", true)]) {
            BigQueryMigration::Statements(s) => assert_eq!(s.len(), 1),
            other => panic!("expected statements, got {:?}", other),
        }
    }

    #[test]
    fn array_element_widening_is_refused() {
        let why = reason(&[SchemaOperation::WidenColumnType {
            name: "tags".into(),
            from: DataType::Array(Box::new(DataType::Integer)),
            to: DataType::Array(Box::new(DataType::BigInt)),
        }]);
        assert!(why.contains("tags") && why.contains("ARRAY"), "{why}");
    }

    #[test]
    fn nullability_relaxes_but_never_tightens() {
        assert_eq!(
            stmts(&[SchemaOperation::ChangeNullability {
                name: "amount".into(),
                to_nullable: true,
                default_expr: None,
            }]),
            vec!["ALTER TABLE `ds.t` ALTER COLUMN `amount` DROP NOT NULL"]
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
            vec!["ALTER TABLE `ds.t` DROP COLUMN `amount`"]
        );
    }

    #[test]
    fn every_nested_operation_is_refused_by_name() {
        let cases: Vec<(SchemaOperation, &str)> = vec![
            (
                SchemaOperation::AddStructField {
                    column: "meta".into(),
                    path: vec![],
                    field_name: "b".into(),
                    field_type: DataType::Integer,
                    default_expr: None,
                },
                "meta",
            ),
            (
                SchemaOperation::RemoveStructField {
                    column: "meta".into(),
                    path: vec![],
                    field_name: "b".into(),
                },
                "meta",
            ),
            (
                SchemaOperation::WidenNestedType {
                    column: "meta".into(),
                    path: vec!["a".into()],
                    from: DataType::Integer,
                    to: DataType::BigInt,
                },
                "meta",
            ),
            (
                SchemaOperation::RewriteColumn {
                    column: "meta".into(),
                    target_type: DataType::Integer,
                    using_expr: "CAST(meta AS INT64)".into(),
                },
                "meta",
            ),
        ];
        for (op, needle) in cases {
            let why = reason(std::slice::from_ref(&op));
            assert!(
                why.contains(needle),
                "the refusal for {:?} must name the column: {why}",
                op
            );
        }
    }

    #[test]
    fn backfill_update_carries_a_where_clause() {
        // BigQuery rejects an UPDATE with no WHERE.
        assert_eq!(
            stmts(&[SchemaOperation::BackfillColumn {
                name: "amount".into(),
                expression: "0".into(),
            }]),
            vec!["UPDATE `ds.t` SET `amount` = 0 WHERE TRUE"]
        );
    }

    #[test]
    fn a_project_qualified_schema_stays_inside_one_backtick_pair() {
        // `project.dataset` must not become `project`.`dataset` — a hyphenated
        // project id needs the quoting, and the whole path form carries it.
        match generate_bigquery_ddl(
            "smelt-bq-test.ds",
            "t",
            &[SchemaOperation::RemoveColumn {
                name: "amount".into(),
            }],
            &[],
        ) {
            BigQueryMigration::Statements(s) => assert_eq!(
                s,
                vec!["ALTER TABLE `smelt-bq-test.ds.t` DROP COLUMN `amount`"]
            ),
            other => panic!("expected statements, got {:?}", other),
        }
    }
}
