//! Shared "run smelt inference, compare against a live oracle" logic used by
//! every dual-target (DuckDB/Spark) type property test.
//!
//! Factored out of `type_property_tests.rs` so `prop_numeric_function_types.rs`
//! (and any future targeted property test) can reuse the same comparison path
//! instead of re-implementing it.

use std::collections::HashMap;

use smelt_db::type_inference::{infer_select_column_types, TypeContext};
use smelt_parser::ast::File;
use smelt_types::{DataType, TypedColumn};

use super::divergences::{find_divergence, TypeDivergence};
use super::generators::{TypedExpr, TypedSource};
use super::known_unknowns::{find_known_unknown, KnownUnknown};
use smelt_oracle_testkit::{
    classify_oracle_error, compare_types, OracleErrorKind, TypeMatch, TypeOracle,
};

/// What `check_types_against_oracle` actually verified for one generated
/// case. Exists so a caller can accumulate coverage across many cases and
/// assert a floor (see the BigQuery leg in `type_property_tests.rs`) — a leg
/// that "ran" every case but compared zero columns, because every case was
/// skipped as a refusal, is indistinguishable from a healthy run unless
/// something counts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OracleCheckOutcome {
    /// Number of output columns whose smelt-inferred type was actually
    /// compared against the oracle's reported type this call (includes
    /// known-unknown and registered-divergence matches — anything that
    /// reached the comparison logic, not just exact matches).
    pub columns_compared: usize,
    /// True when the whole case was skipped because the oracle refused the
    /// generated SQL outright — comparison never started.
    pub query_refused: bool,
}

/// Parse SQL with smelt and run type inference on each select column.
pub fn run_smelt_inference(sql: &str, columns: &[TypedSource]) -> Vec<(String, DataType)> {
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

/// Build an alias → generating-SQL lookup so an inferred `Unknown` column can be
/// matched against the known-unknowns registry by its generating expression.
pub fn expr_sql_by_alias(exprs: &[TypedExpr]) -> HashMap<String, String> {
    exprs
        .iter()
        .map(|e| (e.alias.clone(), e.sql.clone()))
        .collect()
}

/// Compare smelt inference against one oracle backend, returning an error message on mismatch.
///
/// Returns `Err` both for a genuine type mismatch and for a `Fatal` oracle
/// failure (see `classify_oracle_error`) — either way the caller's `if let
/// Err(msg) = ...` sites fail the test loudly. A `QueryRefusal` oracle
/// failure returns `Ok` with `query_refused: true` and zero columns
/// compared, matching the previous "skip invalid SQL for this backend"
/// behaviour, but now the skip is visible to the caller instead of silent.
#[allow(clippy::too_many_arguments)]
pub fn check_types_against_oracle(
    oracle: &dyn TypeOracle,
    backend: &str,
    sql: &str,
    columns: &[TypedSource],
    exprs: &[TypedExpr],
    divergences: &[TypeDivergence],
    unknowns: &[KnownUnknown],
) -> Result<OracleCheckOutcome, String> {
    let actual_types = match oracle.query_types(sql) {
        Ok(types) => types,
        Err(msg) => {
            return match classify_oracle_error(&msg) {
                OracleErrorKind::QueryRefusal => Ok(OracleCheckOutcome {
                    columns_compared: 0,
                    query_refused: true,
                }),
                OracleErrorKind::Fatal => Err(format!(
                    "{backend} oracle unusable — treating as fatal, not a query refusal \
                     (see classify_oracle_error):\n  {msg}\n  SQL: {sql}"
                )),
            };
        }
    };

    let inferred_types = run_smelt_inference(sql, columns);
    let by_alias = expr_sql_by_alias(exprs);

    let mut columns_compared = 0usize;
    for (i, actual) in actual_types.iter().enumerate() {
        let inferred = if i < inferred_types.len() {
            &inferred_types[i]
        } else {
            continue;
        };
        columns_compared += 1;

        let smelt_type = &inferred.1;
        let actual_type = &actual.1;

        if smelt_type.is_unknown() {
            let expr_sql = by_alias.get(&inferred.0).map(String::as_str).unwrap_or(sql);
            if find_known_unknown(expr_sql, unknowns).is_some() {
                continue;
            }
            return Err(format!(
                "Unregistered Unknown inference for column {} ({}) against {backend}:\n  \
                 smelt inferred: {smelt_type:?}\n  \
                 {backend} actual:  {actual_type:?}\n  \
                 generating expr: {expr_sql}\n  \
                 SQL: {sql}\n  \
                 (add a prop_helpers/known_unknowns.rs entry or fix inference)",
                i, actual.0
            ));
        }

        match compare_types(smelt_type, actual_type) {
            TypeMatch::Exact | TypeMatch::Compatible { .. } => {}
            TypeMatch::Mismatch => {
                if find_divergence(smelt_type, actual_type, backend, divergences).is_none() {
                    return Err(format!(
                        "Type mismatch for column {} ({}) against {backend}:\n  \
                         smelt inferred: {smelt_type:?}\n  \
                         {backend} actual:  {actual_type:?}\n  \
                         SQL: {sql}",
                        i, actual.0
                    ));
                }
            }
        }
    }
    Ok(OracleCheckOutcome {
        columns_compared,
        query_refused: false,
    })
}
