//! Hidden decomposed state + presentation view mechanism.
//!
//! See `docs/specs/model_transforms.md` §Semantics "Hidden decomposed state +
//! presentation view" (F12). Given a decomposable aggregate combiner (the
//! `decomposable` discriminant, F4) this derives the hidden monoid-element
//! state columns it decomposes into and a presentation view `π(state)` that
//! recovers the user-facing value from that state row — proven a pure
//! function of exactly those state columns (presentation-map purity, F7).
//!
//! This module states the *mechanism* only: `merge_into` maintaining the
//! state columns and which mode drives it (cumulative rung-2) are decided at
//! the mode-composition layer (`maintenance_plan.md`), not here.

use crate::analysis::discriminants::combiner_discriminants;
use crate::analysis::presentation::{presentation_map_purity, Purity};
use smelt_parser::Expr;
use smelt_types::SqlFunction;

/// One hidden state column backing a decomposed aggregate: its name and the
/// per-partition expression that accumulates it (e.g. `SUM(x)` for the `sum`
/// half of `AVG`'s `(sum, count)` decomposition).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateColumn {
    pub name: String,
    pub per_partition_expr: String,
}

/// The decomposition of one aggregate output into hidden state columns plus
/// a pure presentation view. `(state_columns, presentation_expr)` is one
/// atomically-swapped unit: `merge_into` maintains the state columns; the
/// presentation view is a pure read of that state row (F7), never history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecomposedState {
    pub state_columns: Vec<StateColumn>,
    /// The presentation expression `π(state)`, e.g. `sum / count` for `AVG`.
    pub presentation_expr: String,
}

/// Why an aggregate could not be decomposed to hidden state — refused, never
/// approximated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecomposeRefusal {
    /// The combiner is holistic (`MEDIAN`, exact `COUNT(DISTINCT)`, ...) —
    /// the full multiset is needed to answer it, not a fixed-size state.
    Holistic,
    /// The combiner is decomposable (F4) but this mechanism does not yet
    /// encode its state shape — fail-closed rather than guess at one.
    UnknownStateShape,
    /// The derived presentation expression failed the F7 purity proof.
    ImpurePresentation { reason: String },
}

/// Decompose `function` applied to `arg_expr` (presented as `output_name`)
/// into hidden state + a pure presentation view, or refuse.
///
/// Fail-closed at every step: a holistic combiner (F4 `decomposable ==
/// false`), a decomposable combiner whose state shape this mechanism does
/// not (yet) encode, or a presentation expression that fails the F7 purity
/// proof are all refused — never applied approximately.
pub fn decompose_to_state(
    function: SqlFunction,
    distinct: bool,
    arg_expr: &str,
    output_name: &str,
) -> Result<DecomposedState, DecomposeRefusal> {
    let discriminants = combiner_discriminants(function, distinct);
    if !discriminants.decomposable {
        return Err(DecomposeRefusal::Holistic);
    }

    match function {
        SqlFunction::Avg => {
            let sum_col = format!("{output_name}__sum");
            let count_col = format!("{output_name}__count");
            let state_columns = vec![
                StateColumn {
                    name: sum_col.clone(),
                    per_partition_expr: format!("SUM({arg_expr})"),
                },
                StateColumn {
                    name: count_col.clone(),
                    per_partition_expr: format!("COUNT({arg_expr})"),
                },
            ];
            let presentation_text = format!("{sum_col} / {count_col}");
            build_decomposed_state(state_columns, presentation_text)
        }
        // Decomposable per F4 (variance/stddev family, approx-distinct) but
        // this mechanism does not yet encode a concrete state shape for
        // them — fail-closed rather than guess at a decomposition.
        _ => Err(DecomposeRefusal::UnknownStateShape),
    }
}

/// Assemble a `DecomposedState` from already-derived `state_columns` and a
/// presentation expression, verifying the F7 purity proof before accepting
/// it. Factored out of [`decompose_to_state`] so the fail-closed purity gate
/// is directly testable independent of any one combiner's state shape.
fn build_decomposed_state(
    state_columns: Vec<StateColumn>,
    presentation_text: String,
) -> Result<DecomposedState, DecomposeRefusal> {
    let state_names: Vec<String> = state_columns.iter().map(|c| c.name.clone()).collect();
    // A presentation expression this parser cannot even resolve to an `Expr`
    // is refused the same as an impure one — never assumed pure by default.
    let Some(expr) = parse_presentation_expr(&presentation_text) else {
        return Err(DecomposeRefusal::UnknownStateShape);
    };
    match presentation_map_purity(&expr, &state_names) {
        Purity::Pure => Ok(DecomposedState {
            state_columns,
            presentation_expr: presentation_text,
        }),
        Purity::Impure { reason } => Err(DecomposeRefusal::ImpurePresentation { reason }),
    }
}

fn parse_presentation_expr(text: &str) -> Option<Expr> {
    let sql = format!("SELECT {text} AS v FROM state_row");
    let parse = smelt_parser::parse(&sql);
    let file = smelt_parser::File::cast(parse.syntax())?;
    file.select_stmt()
        .and_then(|select| select.select_list())
        .and_then(|list| list.items().next())
        .and_then(|item| item.expression())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn avg_decomposes_to_sum_count_state_with_pure_view() {
        let decomposed = decompose_to_state(SqlFunction::Avg, false, "amount", "avg_amount")
            .expect("AVG should decompose");
        assert_eq!(
            decomposed.state_columns,
            vec![
                StateColumn {
                    name: "avg_amount__sum".to_string(),
                    per_partition_expr: "SUM(amount)".to_string(),
                },
                StateColumn {
                    name: "avg_amount__count".to_string(),
                    per_partition_expr: "COUNT(amount)".to_string(),
                },
            ]
        );
        assert_eq!(
            decomposed.presentation_expr,
            "avg_amount__sum / avg_amount__count"
        );
    }

    #[test]
    fn holistic_combiner_is_refused() {
        let result = decompose_to_state(SqlFunction::Median, false, "amount", "median_amount");
        assert_eq!(result, Err(DecomposeRefusal::Holistic));
    }

    #[test]
    fn exact_distinct_is_refused_as_holistic() {
        // COUNT(DISTINCT x) is holistic (F4), even though plain COUNT is
        // classified separately (a monoid, not decomposable either — but
        // for a different reason). Decomposition must still refuse it.
        let result = decompose_to_state(SqlFunction::Count, true, "user_id", "distinct_users");
        assert_eq!(result, Err(DecomposeRefusal::Holistic));
    }

    #[test]
    fn decomposable_combiner_without_known_state_shape_is_refused() {
        // Variance is decomposable per F4 but this mechanism has no encoded
        // state shape for it yet — must fail closed, never guess.
        let result = decompose_to_state(SqlFunction::Variance, false, "amount", "var_amount");
        assert_eq!(result, Err(DecomposeRefusal::UnknownStateShape));
    }

    #[test]
    fn impure_presentation_expression_is_refused() {
        // Exercise the purity gate directly: a presentation expression that
        // reaches outside the state row (per F7) must be refused even
        // though its state-column shape is otherwise well-formed.
        let state_columns = vec![StateColumn {
            name: "avg_amount__sum".to_string(),
            per_partition_expr: "SUM(amount)".to_string(),
        }];
        let result = build_decomposed_state(state_columns, "other_table.amount".to_string());
        assert!(matches!(
            result,
            Err(DecomposeRefusal::ImpurePresentation { .. })
        ));
    }
}
