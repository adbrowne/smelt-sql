//! Additive-only model-diff.
//!
//! See `docs/specs/model_properties.md` §"Derived proofs" → "Additive-only
//! model-diff". A model edit is admissible for an in-place backfill (rather
//! than a full rebuild) only when it *purely adds* columns whose
//! dependencies are derivable from `{existing target columns} ∪ {monotone
//! dimension}`. Whether an *existing* column's semantics changed is **not**
//! derivable from the column/dependency-set diff alone — that residue falls
//! to a declared migration intent, out of scope here (L3).

use smelt_parser::{Expr, SyntaxKind};
use std::collections::HashSet;

/// One column of a model version: its name and the expression that computes
/// it.
#[derive(Debug, Clone)]
pub struct ColumnDef {
    pub name: String,
    pub expr: Expr,
}

/// Verdict for whether a model edit is a pure column addition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelDiff {
    /// The edit only adds columns; every added column's dependencies are
    /// derivable from `{existing target} ∪ {monotone dimension}`. An
    /// in-place backfill is admissible.
    AdditiveOnly,
    /// The edit changes or drops an existing column, or adds a column whose
    /// dependencies are not derivable from the existing target + monotone
    /// dimension. A rebuild (or a declared migration) is required.
    NotAdditive { reason: String },
}

impl ModelDiff {
    pub fn is_additive_only(&self) -> bool {
        matches!(self, ModelDiff::AdditiveOnly)
    }
}

/// Prove (or refuse) that `new_columns` is a purely additive edit of
/// `old_columns`, where an added column may depend only on `old_columns`'
/// names plus `monotone_dims`.
///
/// Fail-closed (`model_properties.md` §Constraints): a dropped/renamed
/// column, a changed existing-column expression, a new column depending on
/// something outside `{existing} ∪ {monotone dim}`, or an unresolvable
/// dependency (opaque function, subquery, window) yields `NotAdditive`,
/// never an optimistic `AdditiveOnly`.
pub fn additive_only_diff(
    old_columns: &[ColumnDef],
    new_columns: &[ColumnDef],
    monotone_dims: &[String],
) -> ModelDiff {
    let new_by_name: std::collections::HashMap<&str, &ColumnDef> =
        new_columns.iter().map(|c| (c.name.as_str(), c)).collect();

    for old in old_columns {
        match new_by_name.get(old.name.as_str()) {
            None => {
                return ModelDiff::NotAdditive {
                    reason: format!(
                        "column '{}' is absent from the new model — a drop or rename cannot be \
                         derived as a pure addition",
                        old.name
                    ),
                };
            }
            Some(new_col) => {
                if old.expr.text().trim() != new_col.expr.text().trim() {
                    return ModelDiff::NotAdditive {
                        reason: format!(
                            "existing column '{}' changed its expression — a rebuild or declared \
                             migration is required",
                            old.name
                        ),
                    };
                }
            }
        }
    }

    let existing_names: HashSet<&str> = old_columns.iter().map(|c| c.name.as_str()).collect();
    let monotone_set: HashSet<&str> = monotone_dims.iter().map(|s| s.as_str()).collect();

    for new_col in new_columns {
        if existing_names.contains(new_col.name.as_str()) {
            continue; // already checked above as an existing column
        }
        let deps = match collect_dependencies(&new_col.expr) {
            Ok(deps) => deps,
            Err(reason) => {
                return ModelDiff::NotAdditive {
                    reason: format!(
                        "new column '{}' has an unresolvable dependency: {reason}",
                        new_col.name
                    ),
                };
            }
        };
        for dep in &deps {
            if !existing_names.contains(dep.as_str()) && !monotone_set.contains(dep.as_str()) {
                return ModelDiff::NotAdditive {
                    reason: format!(
                        "new column '{}' depends on '{dep}', which is neither an existing target \
                         column nor a declared monotone dimension",
                        new_col.name
                    ),
                };
            }
        }
    }

    ModelDiff::AdditiveOnly
}

/// Collect the set of column names `expr` depends on, walking its known
/// pure-scalar/aggregate shapes. Any shape not explicitly recognised here (a
/// subquery, a window `OVER`, an opaque/unrecognised construct) fails closed
/// with a reason rather than guessing at its dependencies.
fn collect_dependencies(expr: &Expr) -> Result<HashSet<String>, String> {
    if expr
        .syntax()
        .descendants()
        .any(|n| n.kind() == SyntaxKind::SUBQUERY)
    {
        return Err("references a subquery".to_string());
    }
    if expr.window_spec().is_some()
        || expr
            .syntax()
            .descendants()
            .any(|n| n.kind() == SyntaxKind::WINDOW_SPEC)
    {
        return Err("contains a window OVER clause".to_string());
    }

    let mut deps = HashSet::new();
    walk(expr, &mut deps)?;
    Ok(deps)
}

fn walk(expr: &Expr, deps: &mut HashSet<String>) -> Result<(), String> {
    if let Some(col) = expr.as_column_ref() {
        deps.insert(col.name().to_string());
        return Ok(());
    }

    if let Some(func) = expr.as_function_call() {
        for arg in func.arguments() {
            walk(&arg, deps)?;
        }
        return Ok(());
    }

    if let Some(bin) = expr.as_binary() {
        for side in [bin.left(), bin.right()].into_iter().flatten() {
            walk(&side, deps)?;
        }
        return Ok(());
    }

    if let Some(case) = expr.as_case() {
        if let Some(value) = case.case_value() {
            walk(&value, deps)?;
        }
        for when in case.when_clauses() {
            for arm in [when.condition(), when.result()].into_iter().flatten() {
                walk(&arm, deps)?;
            }
        }
        if let Some(else_expr) = case.else_expr() {
            walk(&else_expr, deps)?;
        }
        return Ok(());
    }

    if let Some(cast) = expr.as_cast() {
        return match cast.expression() {
            Some(inner) => walk(&inner, deps),
            None => Err("CAST has no inner expression to resolve".to_string()),
        };
    }

    // A leaf with no identifier tokens at all is a literal (number, string,
    // NULL, boolean) — no dependency.
    let has_ident = expr
        .syntax()
        .descendants_with_tokens()
        .filter_map(|e| e.into_token())
        .any(|t| t.kind() == SyntaxKind::IDENT);
    if !has_ident {
        return Ok(());
    }

    // Any other shape (BETWEEN, IN, EXISTS, array/struct literals, etc.) is
    // not yet classified here — fail closed rather than guess.
    Err("expression shape is not recognised by the additive-only model-diff proof".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn col(name: &str, sql_expr: &str) -> ColumnDef {
        let sql = format!("SELECT {sql_expr} AS v FROM t");
        let parse = smelt_parser::parse(&sql);
        let file = smelt_parser::File::cast(parse.syntax()).expect("file");
        let select = file.select_stmt().expect("select");
        let item = select
            .select_list()
            .expect("select list")
            .items()
            .next()
            .expect("item");
        ColumnDef {
            name: name.to_string(),
            expr: item.expression().expect("expression"),
        }
    }

    #[test]
    fn pure_addition_from_existing_and_monotone_dim_is_additive_only() {
        let old = vec![col("amount", "amount"), col("user_id", "user_id")];
        let new = vec![
            col("amount", "amount"),
            col("user_id", "user_id"),
            col("amount_usd", "amount * fx_rate"),
        ];
        let verdict = additive_only_diff(&old, &new, &["fx_rate".to_string()]);
        assert_eq!(verdict, ModelDiff::AdditiveOnly);
    }

    #[test]
    fn changed_existing_column_expression_is_not_additive() {
        let old = vec![col("amount_usd", "amount * fx_rate")];
        let new = vec![col("amount_usd", "amount * fx_rate * 2")];
        let verdict = additive_only_diff(&old, &new, &["fx_rate".to_string()]);
        assert!(!verdict.is_additive_only());
    }

    #[test]
    fn new_column_with_non_monotone_new_dependency_is_not_additive() {
        let old = vec![col("amount", "amount")];
        let new = vec![
            col("amount", "amount"),
            col("amount_usd", "amount * fx_rate"),
        ];
        // fx_rate is neither an existing target column nor a declared
        // monotone dimension.
        let verdict = additive_only_diff(&old, &new, &[]);
        assert!(!verdict.is_additive_only());
    }

    #[test]
    fn dropped_or_renamed_column_fails_closed() {
        let old = vec![col("amount", "amount"), col("legacy_total", "amount * 2")];
        // legacy_total is gone; total_v2 appears with the same expression —
        // this looks like a rename, but the diff cannot derive that
        // correspondence, so it must refuse rather than guess.
        let new = vec![col("amount", "amount"), col("total_v2", "amount * 2")];
        let verdict = additive_only_diff(&old, &new, &[]);
        assert!(!verdict.is_additive_only());
    }

    #[test]
    fn unresolvable_dependency_in_new_column_fails_closed() {
        let old = vec![col("amount", "amount")];
        let new = vec![
            col("amount", "amount"),
            col("amount_ranked", "RANK() OVER (ORDER BY amount)"),
        ];
        let verdict = additive_only_diff(&old, &new, &[]);
        assert!(!verdict.is_additive_only());
    }

    #[test]
    fn no_op_edit_is_additive_only() {
        let old = vec![col("amount", "amount")];
        let new = vec![col("amount", "amount")];
        let verdict = additive_only_diff(&old, &new, &[]);
        assert_eq!(verdict, ModelDiff::AdditiveOnly);
    }
}
