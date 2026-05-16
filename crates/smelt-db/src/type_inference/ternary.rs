//! Ternary expression type inference for the meta-language (Phase F).
//!
//! `if COND then THEN_EXPR else ELSE_EXPR` is a pure meta-world construct.
//! It does NOT produce SQL `CASE WHEN`; placement in a Data-World position
//! emits `TernaryInDataPosition` (detected at splice time by Phase 3's
//! `check_file_diagnostics`; the pure-inference level here cannot always
//! determine the parent context).
//!
//! Pure-function rule (CLAUDE.md): no Salsa imports.

use smelt_parser::ast::TernaryExpr;
use smelt_types::{
    signatures::{SmeltType, TypeConstraint},
    DataType, TypedColumn,
};

use super::dispatch::{infer_expression_type, promote_types};
use super::type_context::TypeContext;

/// Pending diagnostic sentinel produced by [`infer_ternary_type`].
///
/// Phase 3 converts these to `Diagnostic` records anchored at the correct spans.
/// Phase 2 produces the sentinels during pure inference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TernarySentinel {
    /// The condition expression synthesised a non-Boolean type.
    ConditionNotBoolean { found: SmeltType },
    /// The then-branch and else-branch have incompatible types.
    BranchTypeMismatch {
        then_type: SmeltType,
        else_type: SmeltType,
    },
}

/// Short-circuit hint emitted by [`infer_ternary_type`] when the condition
/// is a compile-time static Boolean literal.
///
/// Phase 3 uses this sentinel to suppress *evaluation* diagnostics (e.g.
/// `MapGetMissingKey`) that originate from the statically-unreached branch.
/// *Type-checking* diagnostics (e.g. `BranchTypeMismatch`) are always emitted
/// by Phase 2 regardless of this hint — correctness must hold in both branches
/// even when one is unreachable at runtime.
///
/// The sentinel is `None` when the condition is a runtime expression whose
/// value cannot be resolved at compile time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShortCircuitHint {
    /// Condition is the literal `TRUE` → the THEN branch is always reached,
    /// ELSE is unreachable. Phase 3 should suppress ELSE evaluation diagnostics.
    ThenReached,
    /// Condition is the literal `FALSE` → the ELSE branch is always reached,
    /// THEN is unreachable. Phase 3 should suppress THEN evaluation diagnostics.
    ElseReached,
}

/// Result of [`infer_ternary_type`].
///
/// Combines the inferred type, any type-checking sentinels, and the
/// short-circuit hint (set when the condition is a compile-time Boolean literal).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TernaryResult {
    /// The synthesised type of the ternary expression (`Unknown` on error).
    pub ty: SmeltType,
    /// Type-checking sentinels. Phase 3 converts these to `Diagnostic` records.
    pub sentinels: Vec<TernarySentinel>,
    /// Short-circuit hint for Phase 3's evaluation-diagnostic suppression.
    ///
    /// `Some(ThenReached)` — condition is literally `TRUE`; suppress ELSE eval diagnostics.
    /// `Some(ElseReached)` — condition is literally `FALSE`; suppress THEN eval diagnostics.
    /// `None` — condition value unknown at compile time; no suppression.
    pub short_circuit: Option<ShortCircuitHint>,
}

/// Infer the type of a ternary `if COND then THEN else ELSE` expression (Phase F).
///
/// Algorithm:
/// 1. Synthesise the condition type. If not Boolean (or Unknown), record
///    `ConditionNotBoolean` sentinel and return `TernaryResult { Unknown, [sentinel], None }`.
/// 2. Detect a compile-time Boolean literal in the condition to populate
///    `short_circuit` (`Some(ThenReached)` for `TRUE`, `Some(ElseReached)` for
///    `FALSE`, `None` for runtime expressions). Phase 3 uses this to suppress
///    evaluation diagnostics from the statically-unreached branch.
/// 3. Synthesise both branch types independently under the surrounding context.
/// 4. Compute the LUB (least-upper-bound) of the two branch types:
///    - Identical types → that type.
///    - Both `Expr<T>` → use `promote_types` on the inner `DataType`.
///    - Incompatible → `Unknown` + `BranchTypeMismatch` sentinel.
/// 5. Return `TernaryResult { lub_type, sentinels, short_circuit }`.
///
/// Short-circuit contract:
/// - Phase 2 (this function) always type-checks BOTH branches, even when the
///   condition is a static literal. `BranchTypeMismatch` fires regardless.
/// - Phase 2 emits `short_circuit` so Phase 3 can suppress *evaluation*
///   diagnostics (e.g. `MapGetMissingKey`) from the unreached branch.
/// - Phase 3 is the sole consumer of `short_circuit`; Phase 2 only produces it.
///
/// Pure function — no Salsa dependency.
pub fn infer_ternary_type(ternary: &TernaryExpr, ctx: &TypeContext) -> TernaryResult {
    let mut sentinels = Vec::new();

    // Step 1: Synthesise condition type.
    let (cond_ty, short_circuit) = if let Some(cond_expr) = ternary.condition() {
        let ty = match infer_expression_type(&cond_expr, ctx) {
            Some(tc) => SmeltType::Expr(TypeConstraint::Concrete(tc.data_type)),
            None => SmeltType::Unknown,
        };
        // Step 2: Detect compile-time Boolean literal for short-circuit hint.
        let hint = detect_static_bool_literal(&cond_expr);
        (ty, hint)
    } else {
        (SmeltType::Unknown, None)
    };

    let cond_is_boolean = match &cond_ty {
        SmeltType::Expr(TypeConstraint::Concrete(DataType::Boolean)) => true,
        SmeltType::Unknown => true, // Unknown propagates — don't double-report
        _ => false,
    };

    if !cond_is_boolean {
        sentinels.push(TernarySentinel::ConditionNotBoolean {
            found: cond_ty.clone(),
        });
        return TernaryResult {
            ty: SmeltType::Unknown,
            sentinels,
            short_circuit: None,
        };
    }

    // Step 3: Synthesise both branch types.
    let then_ty = if let Some(then_expr) = ternary.then_branch() {
        // First try meta-world inference (for nested ternaries, records, etc.)
        let as_smelt = super::multi_model::infer_expression_smelt_type(&then_expr, ctx, None);
        match as_smelt {
            SmeltType::Unknown => {
                // Fall back to scalar inference.
                match infer_expression_type(&then_expr, ctx) {
                    Some(tc) => SmeltType::Expr(TypeConstraint::Concrete(tc.data_type)),
                    None => SmeltType::Unknown,
                }
            }
            other => other,
        }
    } else {
        SmeltType::Unknown
    };

    let else_ty = if let Some(else_expr) = ternary.else_branch() {
        let as_smelt = super::multi_model::infer_expression_smelt_type(&else_expr, ctx, None);
        match as_smelt {
            SmeltType::Unknown => match infer_expression_type(&else_expr, ctx) {
                Some(tc) => SmeltType::Expr(TypeConstraint::Concrete(tc.data_type)),
                None => SmeltType::Unknown,
            },
            other => other,
        }
    } else {
        SmeltType::Unknown
    };

    // Step 4: Compute LUB.
    let lub = compute_ternary_lub(&then_ty, &else_ty);

    if let LubResult::Incompatible = lub {
        sentinels.push(TernarySentinel::BranchTypeMismatch {
            then_type: then_ty,
            else_type: else_ty,
        });
        return TernaryResult {
            ty: SmeltType::Unknown,
            sentinels,
            short_circuit,
        };
    }

    let result_ty = match lub {
        LubResult::Type(ty) => ty,
        LubResult::Incompatible => SmeltType::Unknown,
    };

    TernaryResult {
        ty: result_ty,
        sentinels,
        short_circuit,
    }
}

/// Detect a compile-time static Boolean literal in a condition expression.
///
/// Returns `Some(ShortCircuitHint::ThenReached)` for `TRUE`,
/// `Some(ShortCircuitHint::ElseReached)` for `FALSE`, and `None` for any
/// runtime expression whose value is not statically known.
///
/// Only bare `TRUE` / `FALSE` tokens (case-insensitive) count as static
/// literals. Expressions like `1 = 1` or `NOT FALSE` are runtime and return
/// `None` even though they may always evaluate to a fixed value.
fn detect_static_bool_literal(cond_expr: &smelt_parser::ast::Expr) -> Option<ShortCircuitHint> {
    let text = cond_expr.text().trim().to_string();
    if text.eq_ignore_ascii_case("TRUE") {
        Some(ShortCircuitHint::ThenReached)
    } else if text.eq_ignore_ascii_case("FALSE") {
        Some(ShortCircuitHint::ElseReached)
    } else {
        None
    }
}

/// Result of computing the LUB of two branch types.
enum LubResult {
    /// Both types are compatible; the LUB is this type.
    Type(SmeltType),
    /// The types are incompatible (no common supertype).
    Incompatible,
}

/// Compute the LUB of two ternary branch types.
///
/// Rules:
/// - Identical types (by equality) → that type.
/// - Both `Expr<Concrete(T)>` and `Expr<Concrete(U)>` → `Expr<Concrete(promote(T, U))>`
///   if `promote_types_smelt` succeeds; else `Incompatible`.
/// - Either is `Unknown` → return the other (Unknown propagates without error).
/// - Any other combination → `Incompatible`.
fn compute_ternary_lub(then_ty: &SmeltType, else_ty: &SmeltType) -> LubResult {
    // Identical types.
    if then_ty == else_ty {
        return LubResult::Type(then_ty.clone());
    }

    // Unknown propagates.
    match (then_ty, else_ty) {
        (SmeltType::Unknown, other) => return LubResult::Type(other.clone()),
        (other, SmeltType::Unknown) => return LubResult::Type(other.clone()),
        _ => {}
    }

    // Both Expr<Concrete(T)>: try numeric promotion.
    match (then_ty, else_ty) {
        (
            SmeltType::Expr(TypeConstraint::Concrete(t)),
            SmeltType::Expr(TypeConstraint::Concrete(u)),
        ) => {
            let t_col = TypedColumn {
                data_type: t.clone(),
                nullable: true,
            };
            let u_col = TypedColumn {
                data_type: u.clone(),
                nullable: true,
            };
            let promoted = promote_types(&t_col, &u_col);
            if matches!(promoted.data_type, DataType::Unknown)
                && !matches!(t, DataType::Unknown)
                && !matches!(u, DataType::Unknown)
            {
                // promote_types returned Unknown for genuinely incompatible types.
                LubResult::Incompatible
            } else {
                LubResult::Type(SmeltType::Expr(TypeConstraint::Concrete(
                    promoted.data_type,
                )))
            }
        }
        // Both Expr<Numeric>: compatible.
        (SmeltType::Expr(TypeConstraint::Numeric), SmeltType::Expr(TypeConstraint::Numeric)) => {
            LubResult::Type(SmeltType::Expr(TypeConstraint::Numeric))
        }
        // Expr<Concrete> with Expr<Numeric>: try to see if concrete satisfies Numeric.
        (
            SmeltType::Expr(TypeConstraint::Concrete(t)),
            SmeltType::Expr(TypeConstraint::Numeric),
        )
        | (
            SmeltType::Expr(TypeConstraint::Numeric),
            SmeltType::Expr(TypeConstraint::Concrete(t)),
        ) => {
            if TypeConstraint::Numeric.satisfies(t) {
                LubResult::Type(SmeltType::Expr(TypeConstraint::Numeric))
            } else {
                LubResult::Incompatible
            }
        }
        _ => LubResult::Incompatible,
    }
}

// ─── Unit tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::type_inference::type_context::TypeContext;

    /// Helper: parse `SELECT if COND then THEN else ELSE FROM t` and infer the
    /// ternary type. Returns the full `TernaryResult`.
    fn run(cond_sql: &str, then_sql: &str, else_sql: &str) -> TernaryResult {
        use smelt_parser::ast::File;
        use smelt_parser::SyntaxKind::TERNARY_EXPR;

        let sql = format!(
            "SELECT if {} then {} else {} FROM t",
            cond_sql, then_sql, else_sql
        );
        let parse = smelt_parser::parse(&sql);
        let root = parse.syntax();
        let file = File::cast(root).expect("FILE");
        let select = file.select_stmt().expect("SelectStmt");

        let ternary_node = select
            .syntax()
            .descendants()
            .find(|n| n.kind() == TERNARY_EXPR)
            .expect("TERNARY_EXPR node");
        let ternary = TernaryExpr::cast(ternary_node).expect("TernaryExpr");

        let ctx = TypeContext::new();
        infer_ternary_type(&ternary, &ctx)
    }

    /// Phase 2 emits `ShortCircuitHint::ElseReached` for a `FALSE` condition.
    ///
    /// Contract verified: `if FALSE then m.get('missing') else 'default'` —
    /// the THEN branch is statically unreachable. Phase 2 cannot suppress the
    /// evaluation diagnostic from `m.get` (no Map support at pure-inference
    /// level), but it emits `short_circuit = Some(ElseReached)` so that Phase 3
    /// can suppress evaluation diagnostics (e.g. `MapGetMissingKey`) from the
    /// THEN branch's evaluation.
    ///
    /// This test uses a simpler THEN branch (an integer literal) because
    /// `m.get('missing')` requires Map support that lives in a later phase.
    /// The testable Phase 2 contract is the sentinel value.
    #[test]
    fn ternary_short_circuit_suppresses_unreached_evaluation_diagnostic() {
        // Condition is FALSE → ELSE is always reached; THEN is unreachable.
        let result = run("FALSE", "42", "'default'");
        assert_eq!(
            result.short_circuit,
            Some(ShortCircuitHint::ElseReached),
            "FALSE condition must produce ElseReached short-circuit hint; got: {:?}",
            result.short_circuit,
        );

        // Condition is TRUE → THEN is always reached; ELSE is unreachable.
        let result_true = run("TRUE", "'x'", "'y'");
        assert_eq!(
            result_true.short_circuit,
            Some(ShortCircuitHint::ThenReached),
            "TRUE condition must produce ThenReached short-circuit hint; got: {:?}",
            result_true.short_circuit,
        );

        // Runtime condition → no short-circuit hint.
        let result_runtime = run("some_col = 'value'", "1", "2");
        assert_eq!(
            result_runtime.short_circuit, None,
            "runtime condition must produce no short-circuit hint; got: {:?}",
            result_runtime.short_circuit,
        );
    }
}
