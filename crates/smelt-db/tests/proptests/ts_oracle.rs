//! Smoke tests for TimestampTz type inference against the DuckDB oracle.
//!
//! These tests guard the §16 spec invariant:
//! - `TIMESTAMPTZ` columns infer as `Timestamp { with_timezone: true }`
//! - `NOW()` returns `Timestamp { with_timezone: true }` (non-nullable)

use crate::prop_helpers::duckdb_oracle::{DuckDbOracle, TypeOracle};
use crate::prop_helpers::generators;
use crate::prop_helpers::type_comparison::{compare_types, TypeMatch};

use smelt_db::type_inference::{infer_select_column_types, TypeContext};
use smelt_parser::ast::File;
use smelt_types::{DataType, TypedColumn};

/// Parse SQL with smelt and run type inference on each select column.
fn run_smelt_inference(sql: &str, columns: &[generators::TypedSource]) -> Vec<(String, DataType)> {
    let parse = smelt_parser::parse(sql);
    let root = parse.syntax();
    let file = File::cast(root).expect("failed to cast to File");
    let select_stmt = file.select_stmt().expect("no SelectStmt in parsed SQL");

    let mut ctx = TypeContext::new();
    for col in columns {
        ctx.add_cte_column(
            "data",
            &col.name,
            TypedColumn::nullable(col.data_type.clone()),
        );
    }

    let column_types = infer_select_column_types(&select_stmt, &ctx);

    let select_list = select_stmt.select_list().expect("no select list");
    let items: Vec<_> = select_list.items().collect();

    items
        .iter()
        .zip(column_types.iter())
        .map(|(item, typed_col)| {
            let alias = item.alias().unwrap_or_else(|| "?".to_string());
            (alias, typed_col.data_type.clone())
        })
        .collect()
}

/// Smoke test: a single TIMESTAMPTZ column passes through `SELECT tstz_col` and
/// smelt infers `Timestamp { with_timezone: true }`.
///
/// Both smelt and DuckDB must agree on the tz-aware type.
#[test]
fn timestamptz_column_infers_correctly() {
    let oracle = DuckDbOracle::new();
    let sql = "WITH data AS (SELECT CAST('2024-01-01 12:00:00+00' AS TIMESTAMPTZ) AS tstz_col) \
               SELECT tstz_col AS expr_0 FROM data";

    // DuckDB oracle must return tz-aware timestamp
    let actual = oracle.query_types(sql).unwrap();
    assert_eq!(
        actual[0].1,
        DataType::Timestamp {
            with_timezone: true
        },
        "DuckDB should return Timestamp {{ with_timezone: true }} for TIMESTAMPTZ column; got {:?}",
        actual[0].1
    );

    // smelt inference must also return tz-aware timestamp
    let columns = vec![generators::TypedSource {
        name: "tstz_col".into(),
        data_type: DataType::Timestamp {
            with_timezone: true,
        },
        cast_sql: "CAST('2024-01-01 12:00:00+00' AS TIMESTAMPTZ)".into(),
    }];
    let inferred = run_smelt_inference(sql, &columns);
    assert_eq!(
        inferred[0].1,
        DataType::Timestamp {
            with_timezone: true
        },
        "smelt should infer Timestamp {{ with_timezone: true }} for tstz_col; got {:?}",
        inferred[0].1
    );

    // They must agree exactly
    assert!(
        matches!(
            compare_types(&inferred[0].1, &actual[0].1),
            TypeMatch::Exact | TypeMatch::Compatible { .. }
        ),
        "smelt ({:?}) and DuckDB ({:?}) disagree on TIMESTAMPTZ column type",
        inferred[0].1,
        actual[0].1
    );
}

/// Smoke test: `SELECT NOW()` — smelt must infer `Timestamp { with_timezone: true }`
/// and that must match DuckDB's actual return type.
///
/// This is a regression guard for Phase 1's NOW() implementation.
#[test]
fn now_oracle_matches_duckdb() {
    let oracle = DuckDbOracle::new();
    // A no-column query: NOW() needs no CTE columns, but we wrap in a CTE for
    // consistency with the rest of the oracle harness.
    let sql = "WITH data AS (SELECT CAST(1 AS INTEGER) AS dummy) SELECT NOW() AS expr_0 FROM data";

    // DuckDB returns tz-aware for NOW()
    let actual = oracle.query_types(sql).unwrap();
    assert_eq!(
        actual[0].1,
        DataType::Timestamp {
            with_timezone: true
        },
        "DuckDB should return Timestamp {{ with_timezone: true }} for NOW(); got {:?}",
        actual[0].1
    );

    // smelt must also infer tz-aware for NOW()
    let columns = vec![generators::TypedSource {
        name: "dummy".into(),
        data_type: DataType::Integer,
        cast_sql: "CAST(1 AS INTEGER)".into(),
    }];
    let inferred = run_smelt_inference(sql, &columns);
    assert_eq!(
        inferred[0].1,
        DataType::Timestamp {
            with_timezone: true
        },
        "smelt should infer Timestamp {{ with_timezone: true }} for NOW(); got {:?}",
        inferred[0].1
    );

    // They must agree
    assert!(
        matches!(
            compare_types(&inferred[0].1, &actual[0].1),
            TypeMatch::Exact | TypeMatch::Compatible { .. }
        ),
        "smelt ({:?}) and DuckDB ({:?}) disagree on NOW() return type",
        inferred[0].1,
        actual[0].1
    );
}
