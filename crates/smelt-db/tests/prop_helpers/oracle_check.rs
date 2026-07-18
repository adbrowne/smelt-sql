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
use super::duckdb_oracle::TypeOracle;
use super::generators::{TypedExpr, TypedSource};
use super::known_unknowns::{find_known_unknown, KnownUnknown};
use super::type_comparison::{compare_types, TypeMatch};

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
#[allow(clippy::too_many_arguments)]
pub fn check_types_against_oracle(
    oracle: &dyn TypeOracle,
    backend: &str,
    sql: &str,
    columns: &[TypedSource],
    exprs: &[TypedExpr],
    divergences: &[TypeDivergence],
    unknowns: &[KnownUnknown],
) -> Result<(), String> {
    let actual_types = match oracle.query_types(sql) {
        Ok(types) => types,
        Err(_) => return Ok(()), // Skip invalid SQL for this backend
    };

    let inferred_types = run_smelt_inference(sql, columns);
    let by_alias = expr_sql_by_alias(exprs);

    for (i, actual) in actual_types.iter().enumerate() {
        let inferred = if i < inferred_types.len() {
            &inferred_types[i]
        } else {
            continue;
        };

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
    Ok(())
}
