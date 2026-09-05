//! The one deterministic table every probe runs against.
//!
//! Expressed as a `VALUES` CTE (or, on GoogleSQL, `UNNEST([STRUCT(…)])`) rather
//! than DDL: nothing is materialised, nothing needs cleaning up, and the same
//! text serves a BigQuery dry run and a real execution.
//!
//! Eight rows, one typed column per `TypeConstraint` family, NULLs present in
//! every nullable column. The first row carries an explicit cast on every value
//! so the column types are pinned rather than inferred.

use smelt_types::{DataType, DialectId};

/// Columns, in fixture order. `TypeConstraint` selection in `probe.rs` maps
/// onto exactly these names.
pub const COLUMNS: &[(&str, &str)] = &[
    (
        "rid",
        "row id 1..8, never NULL — the deterministic total order every probe sorts by, \
         and the window frame's ORDER BY. Without it two executions of the same probe \
         can return rows in different orders and the value comparator reports a \
         divergence that is really a missing ORDER BY.",
    ),
    ("g", "grouping key, 2 distinct values"),
    ("n_int", "INTEGER, one NULL"),
    ("n_bigint", "BIGINT, one NULL"),
    ("n_double", "DOUBLE, one NULL, includes a negative"),
    ("n_dec", "DECIMAL(10,2), one NULL"),
    (
        "s_text",
        "VARCHAR, one NULL; never the literal string NULL (Spark rendering)",
    ),
    ("b_bool", "BOOLEAN, one NULL, both values present"),
    ("d_date", "DATE, one NULL"),
    ("ts_ts", "TIMESTAMP, one NULL"),
    ("arr_int", "ARRAY<BIGINT>"),
    ("j_json", "JSON-shaped VARCHAR"),
];

/// A column's per-dialect type name.
fn ty(dialect: DialectId, col: &str) -> &'static str {
    let bq = dialect == DialectId::BigQuery;
    let spark = dialect == DialectId::SparkSql;
    match col {
        "rid" => {
            if bq {
                "INT64"
            } else {
                "BIGINT"
            }
        }
        "g" | "s_text" | "j_json" => {
            if bq || spark {
                "STRING"
            } else {
                "VARCHAR"
            }
        }
        "n_int" => {
            if bq {
                "INT64"
            } else if spark {
                "INT"
            } else {
                "INTEGER"
            }
        }
        "n_bigint" => {
            if bq {
                "INT64"
            } else {
                "BIGINT"
            }
        }
        "n_double" => {
            if bq {
                "FLOAT64"
            } else {
                "DOUBLE"
            }
        }
        "n_dec" => {
            if bq {
                "NUMERIC"
            } else {
                "DECIMAL(10,2)"
            }
        }
        "b_bool" => {
            if bq {
                "BOOL"
            } else {
                "BOOLEAN"
            }
        }
        "d_date" => "DATE",
        "ts_ts" => "TIMESTAMP",
        "arr_int" => {
            if bq {
                "ARRAY<INT64>"
            } else if spark {
                "ARRAY<BIGINT>"
            } else {
                "BIGINT[]"
            }
        }
        other => unreachable!("unknown fixture column {other}"),
    }
}

/// An array literal of `elems`, spelled for `dialect`.
fn array_lit(dialect: DialectId, elems: &str) -> String {
    match dialect {
        DialectId::DuckDb | DialectId::BigQuery => format!("[{elems}]"),
        DialectId::SparkSql => format!("ARRAY({elems})"),
    }
}

/// One cell's literal text, before the cast wrap. `None` is SQL NULL.
type Row = [Option<&'static str>; 12];

/// Eight rows. Read down a column to see its NULL placement; every column that
/// the doc table above calls nullable has exactly one.
const ROWS: &[Row] = &[
    [
        Some("1"),
        Some("'a'"),
        Some("1"),
        Some("10"),
        Some("1.5"),
        Some("1.25"),
        Some("'alpha'"),
        Some("TRUE"),
        Some("DATE '2026-01-01'"),
        Some("TIMESTAMP '2026-01-01 00:00:00'"),
        Some("1, 2"),
        Some(r#"'{"k": 1}'"#),
    ],
    [
        Some("2"),
        Some("'a'"),
        Some("2"),
        Some("20"),
        Some("-2.5"),
        Some("2.50"),
        Some("'beta'"),
        Some("FALSE"),
        Some("DATE '2026-01-02'"),
        Some("TIMESTAMP '2026-01-02 01:00:00'"),
        Some("3"),
        Some(r#"'{"k": 2}'"#),
    ],
    [
        Some("3"),
        Some("'a'"),
        Some("3"),
        Some("30"),
        Some("3.25"),
        Some("3.75"),
        Some("'gamma'"),
        Some("TRUE"),
        Some("DATE '2026-01-03'"),
        Some("TIMESTAMP '2026-01-03 02:00:00'"),
        Some("4, 5, 6"),
        Some(r#"'{"k": 3}'"#),
    ],
    [
        Some("4"),
        Some("'a'"),
        None,
        Some("40"),
        Some("4.0"),
        Some("4.00"),
        Some("'delta'"),
        Some("FALSE"),
        Some("DATE '2026-01-04'"),
        Some("TIMESTAMP '2026-01-04 03:00:00'"),
        Some("7"),
        Some(r#"'{"k": 4}'"#),
    ],
    [
        Some("5"),
        Some("'b'"),
        Some("5"),
        None,
        Some("5.5"),
        Some("5.25"),
        Some("'epsilon'"),
        Some("TRUE"),
        Some("DATE '2026-01-05'"),
        Some("TIMESTAMP '2026-01-05 04:00:00'"),
        Some("8, 9"),
        Some(r#"'{"k": 5}'"#),
    ],
    [
        Some("6"),
        Some("'b'"),
        Some("6"),
        Some("60"),
        None,
        Some("6.50"),
        Some("'zeta'"),
        Some("FALSE"),
        None,
        Some("TIMESTAMP '2026-01-06 05:00:00'"),
        Some("10"),
        Some(r#"'{"k": 6}'"#),
    ],
    [
        Some("7"),
        Some("'b'"),
        Some("7"),
        Some("70"),
        Some("7.5"),
        None,
        None,
        None,
        Some("DATE '2026-01-07'"),
        None,
        Some("11"),
        Some(r#"'{"k": 7}'"#),
    ],
    [
        Some("8"),
        Some("'b'"),
        Some("8"),
        Some("80"),
        Some("8.25"),
        Some("8.75"),
        Some("'theta'"),
        Some("TRUE"),
        Some("DATE '2026-01-08'"),
        Some("TIMESTAMP '2026-01-08 07:00:00'"),
        Some("12, 13"),
        Some(r#"'{"k": 8}'"#),
    ],
];

/// One cell, cast to its column type so the fixture's schema is declared
/// rather than inferred (and so a NULL still carries a type).
fn cell(dialect: DialectId, col_idx: usize, value: Option<&str>) -> String {
    let (col, _) = COLUMNS[col_idx];
    let type_name = ty(dialect, col);
    let literal = match value {
        None => "NULL".to_string(),
        Some(v) if col == "arr_int" => array_lit(dialect, v),
        Some(v) => v.to_string(),
    };
    format!("CAST({literal} AS {type_name})")
}

/// The fixture CTE for `dialect`, ending in a space so a probe query can be
/// concatenated directly onto it.
///
/// GoogleSQL has no `VALUES` table-value constructor, so BigQuery gets
/// `UNNEST([STRUCT(…), …])` — the same eight rows, the same column names.
pub fn fixture_cte(dialect: DialectId) -> String {
    let names: Vec<&str> = COLUMNS.iter().map(|(n, _)| *n).collect();

    if dialect == DialectId::BigQuery {
        let structs: Vec<String> = ROWS
            .iter()
            .enumerate()
            .map(|(row_idx, row)| {
                let cells: Vec<String> = row
                    .iter()
                    .enumerate()
                    .map(|(i, v)| {
                        let c = cell(dialect, i, *v);
                        // Field names come from the first STRUCT; repeating
                        // them on later rows is a GoogleSQL error.
                        if row_idx == 0 {
                            format!("{c} AS {}", names[i])
                        } else {
                            c
                        }
                    })
                    .collect();
                format!("STRUCT({})", cells.join(", "))
            })
            .collect();
        return format!(
            "WITH fixture AS (SELECT * FROM UNNEST([{}])) ",
            structs.join(", ")
        );
    }

    let tuples: Vec<String> = ROWS
        .iter()
        .map(|row| {
            let cells: Vec<String> = row
                .iter()
                .enumerate()
                .map(|(i, v)| cell(dialect, i, *v))
                .collect();
            format!("({})", cells.join(", "))
        })
        .collect();
    format!(
        "WITH fixture AS (SELECT * FROM (VALUES {}) AS t({})) ",
        tuples.join(", "),
        names.join(", ")
    )
}

/// Each fixture column's smelt `DataType`, for building the `TypeContext` the
/// type leg infers against.
///
/// This is the same information `ty()` renders per dialect, in smelt's own
/// vocabulary rather than an engine's. The two are kept in one file precisely
/// so a column cannot be declared one type to the engine and another to
/// inference.
pub fn column_types() -> Vec<(&'static str, DataType)> {
    vec![
        ("rid", DataType::BigInt),
        ("g", DataType::Varchar { max_length: None }),
        ("n_int", DataType::Integer),
        ("n_bigint", DataType::BigInt),
        ("n_double", DataType::Double),
        (
            "n_dec",
            DataType::Decimal {
                precision: 10,
                scale: 2,
            },
        ),
        ("s_text", DataType::Varchar { max_length: None }),
        ("b_bool", DataType::Boolean),
        ("d_date", DataType::Date),
        (
            "ts_ts",
            DataType::Timestamp {
                with_timezone: false,
            },
        ),
        ("arr_int", DataType::Array(Box::new(DataType::BigInt))),
        ("j_json", DataType::Varchar { max_length: None }),
    ]
}

/// How many rows the fixture has. Asserted by the DuckDB execution test rather
/// than assumed.
pub const ROW_COUNT: usize = 8;
