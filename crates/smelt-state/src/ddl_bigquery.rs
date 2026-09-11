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

// ── Warehouse-resident ledger, GoogleSQL spelling ─────────────────────────
//
// The DuckDB builders in `crate::ddl_duckdb` own the ledger's *meaning*
// (`docs/specs/incremental_models.md` §"The frontier record (reconciliation
// ledger)", `docs/specs/incremental_shapes.md` §"The transactional frontier
// write (merge ledger)"); this section owns nothing but its GoogleSQL
// spelling. Same table name, same six columns, same key — a project moved
// from one dialect to the other reads the same ledger shape.
//
// Three GoogleSQL facts make the DuckDB text a hard error rather than a
// dialect wobble, and each is stated as a fact of the realisation in
// `docs/specs/state.md` §"Which dialects realise which structure":
//
// - **Identifiers.** `"schema"` is a *string literal* in GoogleSQL, not an
//   identifier. The whole path is backticked once (`qualified`), the shape
//   `smelt_backend_bigquery::sql::qualified_name` produces, which keeps a
//   schema that already carries a project prefix working. The driver passes
//   only `schema`, so the emitted name is two-part and resolves against the
//   query job's own default project.
// - **`PRIMARY KEY` must say `NOT ENFORCED`** — a bare `PRIMARY KEY` is a
//   syntax error. The declaration is documentation and an optimiser hint;
//   BigQuery never enforces it. Nothing here may rely on the key to refuse a
//   duplicate, which is precisely why the *additive* never-fold-twice refusal
//   — on DuckDB, that constraint violation itself — is not realisable on
//   BigQuery from this module alone.
// - **There is no `ON CONFLICT DO NOTHING`.** The re-run-tolerant upsert is
//   re-expressed as `MERGE … WHEN NOT MATCHED THEN INSERT` against a one-row
//   inline source. `SELECT <literals>` is the source spelling rather than
//   `UNNEST([STRUCT(…)])`: the scaling form matters for a row *set* (a
//   chained `SELECT … UNION ALL …` costs per-row planning, which is why
//   observed-delta row sets use `UNNEST`), and this source is always exactly
//   one row, so the simpler spelling has no cost to avoid.
//
// Kept in `smelt-state` beside its DuckDB twin under the same bookkeeping
// exclusion from the maintenance-plan-purity invariant (`CLAUDE.md`
// §"Maintenance-plan purity" — "ledger DDL/DML in `smelt-state` excluded as
// bookkeeping").

/// The ledger's six columns, in storage order. One list, used by the DDL, the
/// `INSERT` and the `MERGE`, so the three cannot drift out of agreement.
const LEDGER_COLUMNS: [&str; 6] = [
    "model_name",
    "grp",
    "input_name",
    "delta_id",
    "region_start",
    "region_end",
];

/// The four columns that identify one ledger row — the `PRIMARY KEY`, and the
/// `MERGE`'s match condition.
const LEDGER_KEY_COLUMNS: [&str; 4] = ["model_name", "grp", "input_name", "delta_id"];

/// Escape one value for a GoogleSQL single-quoted string literal.
///
/// **Not** `ddl_duckdb`'s quote doubling: `''` does not continue a string in
/// GoogleSQL, and a backslash IS an escape character there (it is not in
/// DuckDB), so a value carrying one would otherwise change meaning. Both are
/// escaped with the documented backslash form.
fn escape_string_literal(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\\'")
}

fn ledger_table(schema: &str) -> String {
    qualified(schema, crate::ddl_duckdb::LEDGER_TABLE_NAME)
}

/// GoogleSQL DDL creating the ledger table if it does not already exist.
/// Idempotent — safe to run before every fold. GoogleSQL counterpart of
/// [`crate::ddl_duckdb::generate_ledger_table_ddl`].
///
/// The `PRIMARY KEY` is declared `NOT ENFORCED` because GoogleSQL has no other
/// form: it documents the row identity and informs the optimiser, and refuses
/// nothing at write time.
pub fn generate_ledger_table_ddl(schema: &str) -> String {
    format!(
        "CREATE TABLE IF NOT EXISTS {} (\
         model_name STRING NOT NULL, \
         grp STRING NOT NULL, \
         input_name STRING NOT NULL, \
         delta_id STRING NOT NULL, \
         region_start STRING NOT NULL, \
         region_end STRING NOT NULL, \
         PRIMARY KEY ({}) NOT ENFORCED)",
        ledger_table(schema),
        LEDGER_KEY_COLUMNS.join(", "),
    )
}

/// GoogleSQL `INSERT` recording one delta identity as folded for `(model,
/// group, input)`. GoogleSQL counterpart of
/// [`crate::ddl_duckdb::generate_ledger_insert_sql`] — with the one semantic
/// difference that matters: it does **not** refuse a repeat, because the
/// table's `PRIMARY KEY` is unenforced. A caller needing the never-fold-twice
/// refusal must obtain it some other way on this dialect.
#[allow(clippy::too_many_arguments)]
pub fn generate_ledger_insert_sql(
    schema: &str,
    model: &str,
    group: &str,
    input: &str,
    delta_id: &str,
    region_start: &str,
    region_end: &str,
) -> String {
    format!(
        "INSERT INTO {} ({}) VALUES ('{}', '{}', '{}', '{}', '{}', '{}')",
        ledger_table(schema),
        LEDGER_COLUMNS.join(", "),
        escape_string_literal(model),
        escape_string_literal(group),
        escape_string_literal(input),
        escape_string_literal(delta_id),
        escape_string_literal(region_start),
        escape_string_literal(region_end),
    )
}

/// GoogleSQL counterpart of
/// [`crate::ddl_duckdb::generate_ledger_upsert_sql`]'s `ON CONFLICT DO
/// NOTHING` — the **bookkeeping** record of a re-run-tolerant
/// (`Grade::Idempotent`) window-forward keyed model's merged window.
///
/// GoogleSQL has no conflict clause, so the no-op-on-repeat behaviour is
/// expressed directly: `MERGE … WHEN NOT MATCHED THEN INSERT` against a
/// one-row inline source, matched on the same four key columns the DuckDB
/// table's `PRIMARY KEY` names. Re-recording an already-recorded window
/// touches nothing — the same observable behaviour, and it does not depend on
/// the unenforced key.
#[allow(clippy::too_many_arguments)]
pub fn generate_ledger_upsert_sql(
    schema: &str,
    model: &str,
    group: &str,
    input: &str,
    delta_id: &str,
    region_start: &str,
    region_end: &str,
) -> String {
    let values = [model, group, input, delta_id, region_start, region_end];
    let source_select = LEDGER_COLUMNS
        .iter()
        .zip(values.iter())
        .map(|(column, value)| format!("'{}' AS {}", escape_string_literal(value), column))
        .collect::<Vec<_>>()
        .join(", ");
    let on_clause = LEDGER_KEY_COLUMNS
        .iter()
        .map(|c| format!("T.{c} = S.{c}"))
        .collect::<Vec<_>>()
        .join(" AND ");
    let insert_values = LEDGER_COLUMNS
        .iter()
        .map(|c| format!("S.{c}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "MERGE {} T USING (SELECT {}) S ON {} WHEN NOT MATCHED THEN INSERT ({}) VALUES ({})",
        ledger_table(schema),
        source_select,
        on_clause,
        LEDGER_COLUMNS.join(", "),
        insert_values,
    )
}

/// The GoogleSQL realisation of the **never-fold-twice record**: insert this
/// delta identity, or touch nothing if it is already there
/// (`docs/specs/incremental_models.md` §Constraints "Never fold a delta
/// already reflected in the state").
///
/// DuckDB realises the same guarantee with
/// [`crate::ddl_duckdb::generate_ledger_insert_sql`] and an *enforced*
/// `PRIMARY KEY`: a repeat raises a constraint violation, and the violation is
/// the refusal. GoogleSQL's `PRIMARY KEY` is `NOT ENFORCED` and raises nothing,
/// so the refusal has to come from the statement's own effect instead — this
/// one inserts zero rows on a repeat, and the caller
/// (`smelt_backend_bigquery::sql::fold_ledger_delta_script`) turns a zero-row
/// outcome into the abort, inside the transaction that also holds the fold.
///
/// The statement itself is [`generate_ledger_upsert_sql`]'s `MERGE … WHEN NOT
/// MATCHED THEN INSERT` verbatim — the two roles differ only in how the caller
/// reads the row count, never in the text, so they are deliberately one
/// builder rather than two spellings that could drift. Delegating also means
/// the additive fold and the idempotent bookkeeping record can never disagree
/// about what "this window is recorded" means.
#[allow(clippy::too_many_arguments)]
pub fn generate_ledger_conditional_insert_sql(
    schema: &str,
    model: &str,
    group: &str,
    input: &str,
    delta_id: &str,
    region_start: &str,
    region_end: &str,
) -> String {
    generate_ledger_upsert_sql(
        schema,
        model,
        group,
        input,
        delta_id,
        region_start,
        region_end,
    )
}

/// GoogleSQL existence check for `(model, group, input, delta_id)` — the
/// best-effort fallback `Backend::fold_ledger_delta` default uses on a backend
/// that cannot wrap the insert and the fold action in one native transaction.
/// GoogleSQL counterpart of
/// [`crate::ddl_duckdb::generate_ledger_exists_sql`].
pub fn generate_ledger_exists_sql(
    schema: &str,
    model: &str,
    group: &str,
    input: &str,
    delta_id: &str,
) -> String {
    format!(
        "SELECT 1 FROM {} WHERE model_name = '{}' AND grp = '{}' AND input_name = '{}' \
         AND delta_id = '{}' LIMIT 1",
        ledger_table(schema),
        escape_string_literal(model),
        escape_string_literal(group),
        escape_string_literal(input),
        escape_string_literal(delta_id),
    )
}

/// GoogleSQL `DELETE` + `INSERT` implementing the ledger's region-recompute
/// reset. GoogleSQL counterpart of
/// [`crate::ddl_duckdb::generate_ledger_recompute_reset_sqls`], with the same
/// half-open intersection test and the same ordering contract (run both, in
/// this order, inside one backend transaction alongside the recompute's own
/// write).
#[allow(clippy::too_many_arguments)]
pub fn generate_ledger_recompute_reset_sqls(
    schema: &str,
    model: &str,
    group: &str,
    region_start: &str,
    region_end: &str,
    input: &str,
    delta_id: &str,
) -> Vec<String> {
    let delete_sql = format!(
        "DELETE FROM {} WHERE model_name = '{}' AND grp = '{}' \
         AND region_start < '{}' AND region_end > '{}'",
        ledger_table(schema),
        escape_string_literal(model),
        escape_string_literal(group),
        escape_string_literal(region_end),
        escape_string_literal(region_start),
    );
    let insert_sql = generate_ledger_insert_sql(
        schema,
        model,
        group,
        input,
        delta_id,
        region_start,
        region_end,
    );
    vec![delete_sql, insert_sql]
}

#[cfg(test)]
mod ledger_tests {
    use super::*;

    #[test]
    fn the_ddl_declares_a_not_enforced_primary_key_over_a_backticked_two_part_name() {
        assert_eq!(
            generate_ledger_table_ddl("smelt_dogfood"),
            "CREATE TABLE IF NOT EXISTS `smelt_dogfood._smelt_ledger` (\
             model_name STRING NOT NULL, grp STRING NOT NULL, input_name STRING NOT NULL, \
             delta_id STRING NOT NULL, region_start STRING NOT NULL, region_end STRING NOT NULL, \
             PRIMARY KEY (model_name, grp, input_name, delta_id) NOT ENFORCED)"
        );
    }

    #[test]
    fn the_ddl_carries_no_duckdb_type_name_and_no_double_quoted_identifier() {
        let ddl = generate_ledger_table_ddl("ds");
        assert!(!ddl.contains("VARCHAR"), "{ddl}");
        assert!(!ddl.contains('"'), "{ddl}");
    }

    #[test]
    fn the_insert_lists_its_columns_and_backticks_the_table() {
        assert_eq!(
            generate_ledger_insert_sql(
                "ds",
                "silver.events_deduped",
                "{*}",
                "smelt.raw_events",
                "2026-01-01",
                "2026-01-01",
                "2026-01-02",
            ),
            "INSERT INTO `ds._smelt_ledger` \
             (model_name, grp, input_name, delta_id, region_start, region_end) \
             VALUES ('silver.events_deduped', '{*}', 'smelt.raw_events', '2026-01-01', \
             '2026-01-01', '2026-01-02')"
        );
    }

    /// The row's named defect: GoogleSQL has no `ON CONFLICT DO NOTHING`.
    #[test]
    fn the_upsert_is_a_merge_when_not_matched_and_never_on_conflict() {
        let sql = generate_ledger_upsert_sql(
            "ds",
            "m",
            "{*}",
            "smelt.src",
            "2026-01-01",
            "2026-01-01",
            "2026-01-02",
        );
        assert_eq!(
            sql,
            "MERGE `ds._smelt_ledger` T USING (SELECT 'm' AS model_name, '{*}' AS grp, \
             'smelt.src' AS input_name, '2026-01-01' AS delta_id, '2026-01-01' AS region_start, \
             '2026-01-02' AS region_end) S ON T.model_name = S.model_name AND T.grp = S.grp AND \
             T.input_name = S.input_name AND T.delta_id = S.delta_id WHEN NOT MATCHED THEN \
             INSERT (model_name, grp, input_name, delta_id, region_start, region_end) \
             VALUES (S.model_name, S.grp, S.input_name, S.delta_id, S.region_start, S.region_end)"
        );
        assert!(!sql.contains("ON CONFLICT"), "{sql}");
    }

    /// The source is one row, so it needs no `UNNEST([STRUCT(…)])` — and it
    /// must not be a chained `UNION ALL` either.
    #[test]
    fn the_upsert_source_is_a_single_select_row() {
        let sql = generate_ledger_upsert_sql("ds", "m", "g", "i", "d", "s", "e");
        assert!(!sql.contains("UNION ALL"), "{sql}");
        assert_eq!(sql.matches("SELECT").count(), 1, "{sql}");
    }

    #[test]
    fn the_exists_check_matches_the_four_key_columns() {
        assert_eq!(
            generate_ledger_exists_sql("ds", "m", "{*}", "smelt.src", "2026-01-01"),
            "SELECT 1 FROM `ds._smelt_ledger` WHERE model_name = 'm' AND grp = '{*}' \
             AND input_name = 'smelt.src' AND delta_id = '2026-01-01' LIMIT 1"
        );
    }

    #[test]
    fn the_recompute_reset_deletes_every_intersecting_row_then_records_the_input_read() {
        let sqls = generate_ledger_recompute_reset_sqls(
            "ds",
            "m",
            "{*}",
            "2026-01-01",
            "2026-01-03",
            "smelt.src",
            "2026-01-01",
        );
        assert_eq!(sqls.len(), 2);
        assert_eq!(
            sqls[0],
            "DELETE FROM `ds._smelt_ledger` WHERE model_name = 'm' AND grp = '{*}' \
             AND region_start < '2026-01-03' AND region_end > '2026-01-01'"
        );
        assert_eq!(
            sqls[1],
            generate_ledger_insert_sql(
                "ds",
                "m",
                "{*}",
                "smelt.src",
                "2026-01-01",
                "2026-01-01",
                "2026-01-03"
            )
        );
    }

    /// GoogleSQL escaping is backslash-based, not DuckDB's quote doubling —
    /// `''` would terminate the literal here, and an unescaped backslash would
    /// silently change the recorded value.
    #[test]
    fn string_literals_use_googlesql_backslash_escaping() {
        let sql = generate_ledger_insert_sql("ds", "o'brien", "g", "a\\b", "d", "s", "e");
        assert!(sql.contains("'o\\'brien'"), "{sql}");
        assert!(sql.contains("'a\\\\b'"), "{sql}");
        assert!(!sql.contains("o''brien"), "{sql}");
    }

    /// A schema that already carries a project prefix stays one backticked
    /// path, matching `smelt_backend_bigquery::sql::qualified_name`.
    #[test]
    fn a_project_prefixed_schema_stays_one_backticked_path() {
        assert!(generate_ledger_table_ddl("my-proj.ds").contains("`my-proj.ds._smelt_ledger`"));
    }
}
