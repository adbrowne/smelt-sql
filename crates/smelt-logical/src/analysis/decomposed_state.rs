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
//! the mode-composition layer (`incremental_models.md`), not here.

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

/// Decompose `function` applied to `arg_exprs` (presented as `output_name`)
/// into hidden state + a pure presentation view, or refuse.
///
/// `arg_exprs` holds the combiner's argument expressions in call order (one
/// for `AVG`/the variance family, two — value then ordering — for
/// `ArgMax`/`ArgMin`); an arity that does not match the family's shape is a
/// refusal, not a panic.
///
/// Fail-closed at every step: a holistic-or-unknown combiner (F4, the
/// fail-closed discriminant verdict — see
/// [`Discriminants::is_holistic_or_unknown`]), a combiner whose state shape
/// this mechanism does not (yet) encode, or a presentation expression that
/// fails the F7 purity proof are all refused — never applied approximately.
///
/// [`Discriminants::is_holistic_or_unknown`]: crate::analysis::discriminants::Discriminants::is_holistic_or_unknown
pub fn decompose_to_state(
    function: SqlFunction,
    distinct: bool,
    arg_exprs: &[&str],
    output_name: &str,
) -> Result<DecomposedState, DecomposeRefusal> {
    let discriminants = combiner_discriminants(function, distinct);
    if discriminants.is_holistic_or_unknown() {
        return Err(DecomposeRefusal::Holistic);
    }

    match (function, arg_exprs) {
        // `AVG(x)` -> `(sum, count)`, `docs/specs/incremental_models.md`
        // §"Decomposed state (rung 2) in keyed models".
        (SqlFunction::Avg, [arg_expr]) => {
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
            build_decomposed_state(state_columns, presentation_text, &[])
        }

        // The variance/stddev family -> a Welford-style `(n, sx, sxx)`
        // triple, per-family closed form, same spec section.
        (
            SqlFunction::Variance
            | SqlFunction::Stddev
            | SqlFunction::StddevPop
            | SqlFunction::StddevSamp
            | SqlFunction::VarPop
            | SqlFunction::VarSamp,
            [arg_expr],
        ) => decompose_variance_family(function, arg_expr, output_name),

        // `ARG_MAX`/`MAX_BY` (and `ARG_MIN`/`MIN_BY`) -> `(v, o)`, the
        // presented value plus the hidden ordering value, same spec
        // section.
        (SqlFunction::ArgMax, [value_expr, ordering_expr]) => {
            decompose_arg_by(true, value_expr, ordering_expr, output_name)
        }
        (SqlFunction::ArgMin, [value_expr, ordering_expr]) => {
            decompose_arg_by(false, value_expr, ordering_expr, output_name)
        }

        // Decomposable/order-monotone per F4 (approx-distinct's sketch
        // state, or a wrong-arity call to a family above) but this
        // mechanism does not (yet) encode a concrete state shape for it —
        // fail-closed rather than guess at a decomposition.
        _ => Err(DecomposeRefusal::UnknownStateShape),
    }
}

/// The population/sample divisor and its NULL-below-minimum-`n` threshold
/// for one variance/stddev-family function
/// (`docs/specs/incremental_models.md` §"Decomposed state (rung 2) in keyed
/// models"): population divides by `n` and is `NULL` at `n <= 0`; sample
/// divides by `n - 1` and is `NULL` at `n <= 1`. `wrap_sqrt` is set for the
/// `STDDEV_*` forms, not the `VAR*`/`VARIANCE` forms.
fn variance_family_shape(function: SqlFunction) -> Option<(bool, bool)> {
    // (is_sample, wrap_sqrt)
    match function {
        SqlFunction::VarPop => Some((false, false)),
        SqlFunction::StddevPop => Some((false, true)),
        SqlFunction::Variance | SqlFunction::VarSamp => Some((true, false)),
        SqlFunction::Stddev | SqlFunction::StddevSamp => Some((true, true)),
        _ => None,
    }
}

fn decompose_variance_family(
    function: SqlFunction,
    arg_expr: &str,
    output_name: &str,
) -> Result<DecomposedState, DecomposeRefusal> {
    let Some((is_sample, wrap_sqrt)) = variance_family_shape(function) else {
        return Err(DecomposeRefusal::UnknownStateShape);
    };

    let n_col = format!("{output_name}__n");
    let sx_col = format!("{output_name}__sx");
    let sxx_col = format!("{output_name}__sxx");
    let state_columns = vec![
        StateColumn {
            name: n_col.clone(),
            per_partition_expr: format!("COUNT({arg_expr})"),
        },
        StateColumn {
            name: sx_col.clone(),
            per_partition_expr: format!("SUM({arg_expr})"),
        },
        StateColumn {
            name: sxx_col.clone(),
            per_partition_expr: format!("SUM({arg_expr} * {arg_expr})"),
        },
    ];

    let divisor = if is_sample {
        format!("({n_col} * ({n_col} - 1))")
    } else {
        format!("({n_col} * {n_col})")
    };
    let minimum_n = if is_sample { 1 } else { 0 };
    let variance_expr = format!("(({n_col} * {sxx_col}) - ({sx_col} * {sx_col})) / {divisor}");
    let closed_form = if wrap_sqrt {
        format!("SQRT({variance_expr})")
    } else {
        variance_expr
    };
    let presentation_text =
        format!("CASE WHEN {n_col} <= {minimum_n} THEN NULL ELSE {closed_form} END");

    build_decomposed_state(state_columns, presentation_text, &[])
}

/// `ARG_MAX(v, o)` / `ARG_MIN(v, o)` -> `(v, o)` hidden state; `π` presents
/// the bare `v` state column, the ordering value is never presented
/// (`docs/specs/incremental_models.md` §"Decomposed state (rung 2) in keyed
/// models").
fn decompose_arg_by(
    is_max: bool,
    value_expr: &str,
    ordering_expr: &str,
    output_name: &str,
) -> Result<DecomposedState, DecomposeRefusal> {
    let v_col = format!("{output_name}__v");
    let o_col = format!("{output_name}__o");
    let (arg_fn, order_fn) = if is_max {
        ("ARG_MAX", "MAX")
    } else {
        ("ARG_MIN", "MIN")
    };
    let state_columns = vec![
        StateColumn {
            name: v_col.clone(),
            per_partition_expr: format!("{arg_fn}({value_expr}, {ordering_expr})"),
        },
        StateColumn {
            name: o_col,
            per_partition_expr: format!("{order_fn}({ordering_expr})"),
        },
    ];
    build_decomposed_state(state_columns, v_col.clone(), &[])
}

/// One `COALESCE`-first-non-null candidate reduction backing a once-write
/// column's decomposed state — the *bare* per-partition reduction (e.g.
/// `MAX(col)`), never fallback-tainted
/// (`docs/specs/incremental_models.md` §"Decomposed state (rung 2) in keyed
/// models").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnceWriteCandidate {
    /// The bare per-partition reduction expression, e.g. `MAX(col)`.
    pub reduction_expr: String,
}

/// Decompose a once-write column's fallback-bearing or multi-candidate
/// spelling into hidden `(value, written)` state per candidate, plus a
/// presentation view that applies the fallback/preference order fresh on
/// every read (`docs/specs/incremental_models.md` §"Decomposed state (rung
/// 2) in keyed models"). `candidates` is in the caller's declared preference
/// order. `fallback`, if present, is applied only when no candidate is
/// written. `same_row_columns` are columns of the model's own output row
/// the caller vouches are safe for `fallback` to reference (e.g. a
/// same-row default column) — anything else `fallback` reaches is refused
/// as `ImpurePresentation`, never assumed pure.
pub fn decompose_once_write(
    candidates: &[OnceWriteCandidate],
    fallback: Option<&str>,
    same_row_columns: &[String],
    output_name: &str,
) -> Result<DecomposedState, DecomposeRefusal> {
    if candidates.is_empty() {
        return Err(DecomposeRefusal::UnknownStateShape);
    }

    let single = candidates.len() == 1;
    let mut state_columns = Vec::with_capacity(candidates.len() * 2);
    let mut pairs = Vec::with_capacity(candidates.len());
    for (idx, candidate) in candidates.iter().enumerate() {
        let (value_col, written_col) = if single {
            (
                format!("{output_name}__value"),
                format!("{output_name}__written"),
            )
        } else {
            let i = idx + 1;
            (
                format!("{output_name}__value_{i}"),
                format!("{output_name}__written_{i}"),
            )
        };
        state_columns.push(StateColumn {
            name: value_col.clone(),
            per_partition_expr: candidate.reduction_expr.clone(),
        });
        state_columns.push(StateColumn {
            name: written_col.clone(),
            per_partition_expr: format!("({}) IS NOT NULL", candidate.reduction_expr),
        });
        pairs.push((value_col, written_col));
    }

    let presentation_text = match (pairs.as_slice(), fallback) {
        ([(value_col, _written_col)], None) => value_col.clone(),
        ([(value_col, written_col)], Some(fallback)) => {
            format!("CASE WHEN {written_col} THEN {value_col} ELSE {fallback} END")
        }
        (many, fallback) => {
            let mut text = String::from("CASE ");
            for (value_col, written_col) in many {
                text.push_str(&format!("WHEN {written_col} THEN {value_col} "));
            }
            if let Some(fallback) = fallback {
                text.push_str(&format!("ELSE {fallback} "));
            }
            text.push_str("END");
            text
        }
    };

    build_decomposed_state(state_columns, presentation_text, same_row_columns)
}

/// Pure detector for `KeyedStateColumnCollision`: which of `state_columns`
/// collide with a declared or projected user column in `user_columns`.
/// Returns `(state column name, user column name)` pairs; empty when there
/// is no collision. Never renames — smelt does not guess which of the two a
/// consumer meant.
pub fn state_column_collisions(
    state_columns: &[StateColumn],
    user_columns: &[String],
) -> Vec<(String, String)> {
    state_columns
        .iter()
        .flat_map(|state_col| {
            user_columns
                .iter()
                .filter(move |user_col| **user_col == state_col.name)
                .map(move |user_col| (state_col.name.clone(), user_col.clone()))
        })
        .collect()
}

/// Assemble a `DecomposedState` from already-derived `state_columns` and a
/// presentation expression, verifying the F7 purity proof before accepting
/// it. Factored out of [`decompose_to_state`] so the fail-closed purity gate
/// is directly testable independent of any one combiner's state shape.
/// `extra_pure_columns` widens the purity proof's allowed column set beyond
/// the state row's own columns — used only for a once-write fallback's
/// caller-vouched same-row columns; every other caller passes `&[]`.
fn build_decomposed_state(
    state_columns: Vec<StateColumn>,
    presentation_text: String,
    extra_pure_columns: &[String],
) -> Result<DecomposedState, DecomposeRefusal> {
    let mut state_names: Vec<String> = state_columns.iter().map(|c| c.name.clone()).collect();
    state_names.extend(extra_pure_columns.iter().cloned());
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
        let decomposed = decompose_to_state(SqlFunction::Avg, false, &["amount"], "avg_amount")
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
        let result = decompose_to_state(SqlFunction::Median, false, &["amount"], "median_amount");
        assert_eq!(result, Err(DecomposeRefusal::Holistic));
    }

    #[test]
    fn exact_distinct_is_refused_as_holistic() {
        // COUNT(DISTINCT x) is holistic (F4), even though plain COUNT is
        // classified separately (a monoid, not decomposable either — but
        // for a different reason). Decomposition must still refuse it.
        let result = decompose_to_state(SqlFunction::Count, true, &["user_id"], "distinct_users");
        assert_eq!(result, Err(DecomposeRefusal::Holistic));
    }

    #[test]
    fn approx_count_distinct_still_refuses_unknown_state_shape() {
        // Sketch (HLL) state is out of scope for this outcome — decomposable
        // per F4, but must still fail closed rather than guess a shape.
        let result = decompose_to_state(
            SqlFunction::ApproxCountDistinct,
            false,
            &["user_id"],
            "approx_users",
        );
        assert_eq!(result, Err(DecomposeRefusal::UnknownStateShape));
    }

    #[test]
    fn order_monotone_reaches_state_shape_despite_non_decomposable_discriminant() {
        // Pins the entry-gate decision: ArgMax's raw F4 `decomposable` fact
        // stays `false`, yet the state-shape match still succeeds because
        // the gate only refuses the holistic-or-unknown verdict.
        let discriminants = combiner_discriminants(SqlFunction::ArgMax, false);
        assert!(!discriminants.decomposable);

        let result = decompose_to_state(SqlFunction::ArgMax, false, &["v", "o"], "latest_v");
        assert!(result.is_ok(), "expected ArgMax to decompose: {result:?}");
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
        let result = build_decomposed_state(state_columns, "other_table.amount".to_string(), &[]);
        assert!(matches!(
            result,
            Err(DecomposeRefusal::ImpurePresentation { .. })
        ));
    }

    #[test]
    fn variance_family_decomposes_to_n_sx_sxx() {
        for function in [
            SqlFunction::Variance,
            SqlFunction::VarSamp,
            SqlFunction::VarPop,
            SqlFunction::Stddev,
            SqlFunction::StddevPop,
            SqlFunction::StddevSamp,
        ] {
            let decomposed = decompose_to_state(function, false, &["amount"], "stat_amount")
                .unwrap_or_else(|e| panic!("{function:?} should decompose: {e:?}"));
            assert_eq!(
                decomposed.state_columns,
                vec![
                    StateColumn {
                        name: "stat_amount__n".to_string(),
                        per_partition_expr: "COUNT(amount)".to_string(),
                    },
                    StateColumn {
                        name: "stat_amount__sx".to_string(),
                        per_partition_expr: "SUM(amount)".to_string(),
                    },
                    StateColumn {
                        name: "stat_amount__sxx".to_string(),
                        per_partition_expr: "SUM(amount * amount)".to_string(),
                    },
                ],
                "{function:?}"
            );
        }
    }

    #[test]
    fn variance_presentation_uses_family_divisor_and_minimum_n() {
        let pop = decompose_to_state(SqlFunction::VarPop, false, &["amount"], "stat_amount")
            .expect("VAR_POP should decompose");
        assert!(
            pop.presentation_expr.contains("<= 0"),
            "{}",
            pop.presentation_expr
        );
        assert!(
            !pop.presentation_expr.contains("SQRT"),
            "{}",
            pop.presentation_expr
        );

        let samp = decompose_to_state(SqlFunction::VarSamp, false, &["amount"], "stat_amount")
            .expect("VAR_SAMP should decompose");
        assert!(
            samp.presentation_expr.contains("<= 1"),
            "{}",
            samp.presentation_expr
        );
        assert!(
            !samp.presentation_expr.contains("SQRT"),
            "{}",
            samp.presentation_expr
        );

        let stddev_pop =
            decompose_to_state(SqlFunction::StddevPop, false, &["amount"], "stat_amount")
                .expect("STDDEV_POP should decompose");
        assert!(stddev_pop.presentation_expr.contains("<= 0"));
        assert!(stddev_pop.presentation_expr.contains("SQRT"));

        let stddev_samp =
            decompose_to_state(SqlFunction::StddevSamp, false, &["amount"], "stat_amount")
                .expect("STDDEV_SAMP should decompose");
        assert!(stddev_samp.presentation_expr.contains("<= 1"));
        assert!(stddev_samp.presentation_expr.contains("SQRT"));
    }

    #[test]
    fn max_by_decomposes_to_value_and_ordering_state() {
        let decomposed = decompose_to_state(SqlFunction::ArgMax, false, &["v", "o"], "latest_v")
            .expect("MAX_BY should decompose");
        assert_eq!(
            decomposed.state_columns,
            vec![
                StateColumn {
                    name: "latest_v__v".to_string(),
                    per_partition_expr: "ARG_MAX(v, o)".to_string(),
                },
                StateColumn {
                    name: "latest_v__o".to_string(),
                    per_partition_expr: "MAX(o)".to_string(),
                },
            ]
        );
        assert_eq!(decomposed.presentation_expr, "latest_v__v");

        let min_decomposed =
            decompose_to_state(SqlFunction::ArgMin, false, &["v", "o"], "earliest_v")
                .expect("MIN_BY should decompose");
        assert_eq!(
            min_decomposed.state_columns,
            vec![
                StateColumn {
                    name: "earliest_v__v".to_string(),
                    per_partition_expr: "ARG_MIN(v, o)".to_string(),
                },
                StateColumn {
                    name: "earliest_v__o".to_string(),
                    per_partition_expr: "MIN(o)".to_string(),
                },
            ]
        );
    }

    #[test]
    fn once_write_single_reduction_decomposes_to_value_and_written() {
        let candidates = vec![OnceWriteCandidate {
            reduction_expr: "MAX(col)".to_string(),
        }];
        let decomposed = decompose_once_write(&candidates, None, &[], "target")
            .expect("single reduction should decompose");
        assert_eq!(
            decomposed.state_columns,
            vec![
                StateColumn {
                    name: "target__value".to_string(),
                    per_partition_expr: "MAX(col)".to_string(),
                },
                StateColumn {
                    name: "target__written".to_string(),
                    per_partition_expr: "(MAX(col)) IS NOT NULL".to_string(),
                },
            ]
        );
        assert_eq!(decomposed.presentation_expr, "target__value");
    }

    #[test]
    fn once_write_fallback_is_applied_in_presentation_not_merged() {
        let candidates = vec![OnceWriteCandidate {
            reduction_expr: "MAX(col)".to_string(),
        }];
        let decomposed = decompose_once_write(&candidates, Some("0"), &[], "target")
            .expect("fallback-bearing reduction should decompose");
        assert_eq!(
            decomposed.presentation_expr,
            "CASE WHEN target__written THEN target__value ELSE 0 END"
        );
        // The state expression stays the bare reduction — never
        // fallback-tainted.
        assert_eq!(decomposed.state_columns[0].per_partition_expr, "MAX(col)");
    }

    #[test]
    fn once_write_multi_candidate_keeps_one_pair_per_candidate() {
        let candidates = vec![
            OnceWriteCandidate {
                reduction_expr: "MAX(a)".to_string(),
            },
            OnceWriteCandidate {
                reduction_expr: "MAX(b)".to_string(),
            },
        ];
        let decomposed = decompose_once_write(&candidates, None, &[], "target")
            .expect("multi-candidate reduction should decompose");
        assert_eq!(decomposed.state_columns.len(), 4);
        assert_eq!(
            decomposed.state_columns,
            vec![
                StateColumn {
                    name: "target__value_1".to_string(),
                    per_partition_expr: "MAX(a)".to_string(),
                },
                StateColumn {
                    name: "target__written_1".to_string(),
                    per_partition_expr: "(MAX(a)) IS NOT NULL".to_string(),
                },
                StateColumn {
                    name: "target__value_2".to_string(),
                    per_partition_expr: "MAX(b)".to_string(),
                },
                StateColumn {
                    name: "target__written_2".to_string(),
                    per_partition_expr: "(MAX(b)) IS NOT NULL".to_string(),
                },
            ]
        );
        assert_eq!(
            decomposed.presentation_expr,
            "CASE WHEN target__written_1 THEN target__value_1 WHEN target__written_2 THEN target__value_2 END"
        );
    }

    #[test]
    fn once_write_fallback_reaching_outside_the_row_is_refused() {
        let candidates = vec![OnceWriteCandidate {
            reduction_expr: "MAX(col)".to_string(),
        }];
        let result = decompose_once_write(&candidates, Some("other_table.col"), &[], "target");
        assert!(matches!(
            result,
            Err(DecomposeRefusal::ImpurePresentation { .. })
        ));

        // Vouching for the column makes it pure.
        let vouched = decompose_once_write(
            &candidates,
            Some("default_col"),
            &["default_col".to_string()],
            "target",
        );
        assert!(
            vouched.is_ok(),
            "expected vouched fallback to be pure: {vouched:?}"
        );
    }

    #[test]
    fn state_column_collision_is_detected() {
        let state_columns = vec![
            StateColumn {
                name: "total_spend__sum".to_string(),
                per_partition_expr: "SUM(amount)".to_string(),
            },
            StateColumn {
                name: "total_spend__count".to_string(),
                per_partition_expr: "COUNT(amount)".to_string(),
            },
        ];

        let colliding = state_column_collisions(
            &state_columns,
            &["total_spend__sum".to_string(), "customer_id".to_string()],
        );
        assert_eq!(
            colliding,
            vec![(
                "total_spend__sum".to_string(),
                "total_spend__sum".to_string()
            )]
        );

        let none = state_column_collisions(
            &state_columns,
            &["customer_id".to_string(), "total_spend".to_string()],
        );
        assert!(none.is_empty());
    }
}
