//! Presentation-map purity.
//!
//! See `docs/specs/model_properties.md` §"Derived proofs" → "Presentation-map
//! purity" and `docs/specs/model_transforms.md` §Semantics "Hidden decomposed
//! state + presentation view". A hidden-state presentation view (`π(state)`,
//! F12) is sound only when `π` is a **pure function of a single consistent
//! state row** — it must not read any other row, any other table, or a
//! window over other rows. This module states only that soundness
//! condition; it does not decide which columns belong to the state row (the
//! caller supplies `state_columns`) or how the presentation view is wired.

use smelt_parser::{Expr, SyntaxKind};
use smelt_types::SqlFunction;

/// Verdict for whether a presentation-map expression is a pure function of
/// one consistent state row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Purity {
    /// `expr` reads only the state row's own columns plus pure scalar ops.
    Pure,
    /// `expr` reads something other than the state row's own columns —
    /// another table/row, a window, a subquery, or an unresolved/opaque
    /// reference. `reason` names why.
    Impure { reason: String },
}

impl Purity {
    pub fn is_pure(&self) -> bool {
        matches!(self, Purity::Pure)
    }
}

/// Prove (or refuse) that `expr` is a pure function of the state row whose
/// own columns are `state_columns`.
///
/// Fail-closed (`model_properties.md` §Constraints): an unresolved
/// reference, an opaque/unrecognised function, a cross-table column
/// reference, a subquery, or a window `OVER` anywhere in `expr` yields
/// `Impure`, never an optimistic `Pure`.
pub fn presentation_map_purity(expr: &Expr, state_columns: &[String]) -> Purity {
    // Structural checks first: a subquery or a window OVER anywhere in the
    // expression subtree (not just at the top level) disqualifies it,
    // regardless of how deeply it is nested (inside a CASE arm, a function
    // argument, etc.).
    if expr
        .syntax()
        .descendants()
        .any(|n| n.kind() == SyntaxKind::SUBQUERY)
    {
        return Purity::Impure {
            reason: "references a subquery (reads rows beyond the state row)".to_string(),
        };
    }
    if expr.window_spec().is_some()
        || expr
            .syntax()
            .descendants()
            .any(|n| n.kind() == SyntaxKind::WINDOW_SPEC)
    {
        return Purity::Impure {
            reason: "contains a window OVER clause (reads rows beyond the state row)".to_string(),
        };
    }

    walk(expr, state_columns)
}

/// Recursively walk `expr`'s known pure-scalar shapes (column ref, binary
/// op, CASE, CAST, non-aggregate function call). Any shape not explicitly
/// recognised here is treated as opaque and fails closed to `Impure` — this
/// function is never optimistic about a construct it does not understand.
fn walk(expr: &Expr, state_columns: &[String]) -> Purity {
    if let Some(col) = expr.as_column_ref() {
        if col.qualifier().is_some() {
            return Purity::Impure {
                reason: format!(
                    "references column '{}' qualified by another table alias",
                    col.name()
                ),
            };
        }
        return if state_columns.iter().any(|c| c == col.name()) {
            Purity::Pure
        } else {
            Purity::Impure {
                reason: format!(
                    "references column '{}' which is not one of the state row's own columns",
                    col.name()
                ),
            }
        };
    }

    if let Some(func) = expr.as_function_call() {
        let name = func.name().unwrap_or_default().to_uppercase();
        match SqlFunction::from_name(&name) {
            Some(f) if f.is_aggregate() || f.is_window() => {
                return Purity::Impure {
                    reason: format!(
                        "'{name}' is an aggregate/window function — it reads more than one row"
                    ),
                };
            }
            Some(_) => {}
            None => {
                return Purity::Impure {
                    reason: format!("'{name}' is not a recognised scalar function (opaque)"),
                };
            }
        }
        for arg in func.arguments() {
            let verdict = walk(&arg, state_columns);
            if !verdict.is_pure() {
                return verdict;
            }
        }
        return Purity::Pure;
    }

    if let Some(bin) = expr.as_binary() {
        for side in [bin.left(), bin.right()].into_iter().flatten() {
            let verdict = walk(&side, state_columns);
            if !verdict.is_pure() {
                return verdict;
            }
        }
        return Purity::Pure;
    }

    if let Some(case) = expr.as_case() {
        if let Some(value) = case.case_value() {
            let verdict = walk(&value, state_columns);
            if !verdict.is_pure() {
                return verdict;
            }
        }
        for when in case.when_clauses() {
            for arm in [when.condition(), when.result()].into_iter().flatten() {
                let verdict = walk(&arm, state_columns);
                if !verdict.is_pure() {
                    return verdict;
                }
            }
        }
        if let Some(else_expr) = case.else_expr() {
            let verdict = walk(&else_expr, state_columns);
            if !verdict.is_pure() {
                return verdict;
            }
        }
        return Purity::Pure;
    }

    if let Some(cast) = expr.as_cast() {
        return match cast.expression() {
            Some(inner) => walk(&inner, state_columns),
            None => Purity::Impure {
                reason: "CAST has no inner expression to resolve".to_string(),
            },
        };
    }

    // A leaf with no identifier tokens at all is a literal (number, string,
    // NULL, boolean) — pure by construction.
    let has_ident = expr
        .syntax()
        .descendants_with_tokens()
        .filter_map(|e| e.into_token())
        .any(|t| t.kind() == SyntaxKind::IDENT);
    if !has_ident {
        return Purity::Pure;
    }

    // Any other shape (BETWEEN, IN, EXISTS, array/struct literals, etc.) is
    // not yet classified here — fail closed rather than guess.
    Purity::Impure {
        reason: "expression shape is not recognised by the presentation-map purity proof"
            .to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_expr(sql_expr: &str) -> Expr {
        let sql = format!("SELECT {sql_expr} AS v FROM state_row");
        let parse = smelt_parser::parse(&sql);
        let file = smelt_parser::File::cast(parse.syntax()).expect("file");
        let select = file.select_stmt().expect("select");
        let item = select
            .select_list()
            .expect("select list")
            .items()
            .next()
            .expect("item");
        item.expression().expect("expression")
    }

    #[test]
    fn sum_over_state_columns_is_pure() {
        let expr = parse_expr("running_sum + running_count");
        let verdict = presentation_map_purity(
            &expr,
            &["running_sum".to_string(), "running_count".to_string()],
        );
        assert_eq!(verdict, Purity::Pure);
    }

    #[test]
    fn aggregate_function_over_state_columns_is_impure() {
        // An aggregate reads more than one row even if its argument happens
        // to name a state column — the state row is already the fold
        // result, so re-aggregating it is not a pure presentation map.
        let expr = parse_expr("SUM(running_sum)");
        let verdict = presentation_map_purity(&expr, &["running_sum".to_string()]);
        assert!(!verdict.is_pure());
    }

    #[test]
    fn count_alias_over_state_columns_is_pure() {
        let expr =
            parse_expr("CASE WHEN running_count > 0 THEN running_sum / running_count ELSE 0 END");
        let verdict = presentation_map_purity(
            &expr,
            &["running_sum".to_string(), "running_count".to_string()],
        );
        assert_eq!(verdict, Purity::Pure);
    }

    #[test]
    fn cross_table_column_reference_is_impure() {
        let expr = parse_expr("other_table.amount");
        let verdict = presentation_map_purity(&expr, &["amount".to_string()]);
        assert!(!verdict.is_pure());
    }

    #[test]
    fn window_over_clause_is_impure() {
        let expr = parse_expr("SUM(running_sum) OVER (PARTITION BY key ORDER BY ts)");
        let verdict = presentation_map_purity(&expr, &["running_sum".to_string()]);
        assert!(!verdict.is_pure());
    }

    #[test]
    fn subquery_is_impure() {
        let expr = parse_expr("(SELECT MAX(x) FROM other)");
        let verdict = presentation_map_purity(&expr, &["running_sum".to_string()]);
        assert!(!verdict.is_pure());
    }

    #[test]
    fn window_over_nested_inside_case_is_impure() {
        // The window is not at the top level of the expression — it is
        // buried inside a CASE arm. Purity must still catch it: the check
        // is over the whole subtree, not just the top-level node.
        let expr = parse_expr(
            "CASE WHEN running_count > 0 THEN SUM(running_sum) OVER (PARTITION BY key) ELSE 0 END",
        );
        let verdict = presentation_map_purity(
            &expr,
            &["running_sum".to_string(), "running_count".to_string()],
        );
        assert!(!verdict.is_pure());
    }

    #[test]
    fn unresolved_column_reference_fails_closed() {
        let expr = parse_expr("mystery_column");
        let verdict = presentation_map_purity(&expr, &["running_sum".to_string()]);
        assert!(!verdict.is_pure());
    }

    #[test]
    fn opaque_udf_fails_closed() {
        let expr = parse_expr("some_unknown_udf(running_sum)");
        let verdict = presentation_map_purity(&expr, &["running_sum".to_string()]);
        assert!(!verdict.is_pure());
    }

    #[test]
    fn literal_is_pure() {
        let expr = parse_expr("42");
        let verdict = presentation_map_purity(&expr, &["running_sum".to_string()]);
        assert_eq!(verdict, Purity::Pure);
    }
}
