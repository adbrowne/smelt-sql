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

    // Gradual typing: Unknown COND (from an unresolvable expression) is treated
    // as "boolean-OK" — do NOT emit ConditionNotBoolean (that would double-report
    // on top of any diagnostic already emitted for the COND itself).  The result
    // type is the LUB of the two branches, which preserves branch-type-mismatch
    // detection even when the condition cannot be resolved (spec rule 4).
    let cond_is_boolean = matches!(
        cond_ty,
        SmeltType::Expr(TypeConstraint::Concrete(DataType::Boolean)) | SmeltType::Unknown
    );

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
        infer_branch_type(&then_expr, ctx)
    } else {
        SmeltType::Unknown
    };

    let else_ty = if let Some(else_expr) = ternary.else_branch() {
        infer_branch_type(&else_expr, ctx)
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

/// Synthesise the `SmeltType` for one branch of a ternary expression.
///
/// Inference order:
/// 1. If the branch is a nested `TERNARY_EXPR`, recurse via `infer_ternary_type`.
///    This is the fix for the "Unknown hole" bug described in the review: without
///    this check, `infer_expression_smelt_type` and `infer_expression_type` both
///    return `Unknown` for nested ternaries (neither had a `TERNARY_EXPR` dispatch
///    case), causing `compute_ternary_lub` to silently accept the Unknown and
///    suppress `BranchTypeMismatch`.
/// 2. Try meta-world inference (handles pipes, HOF calls, list literals, records).
/// 3. Fall back to scalar SQL inference.
///
/// Note: when a nested ternary is detected, its own sentinels are NOT forwarded
/// to the outer ternary — the nested ternary's errors are emitted by
/// `check_ternary_expr_diagnostics` when it independently walks the nested node.
/// The outer call only needs the synthesised result type to compute the outer LUB.
fn infer_branch_type(expr: &smelt_parser::ast::Expr, ctx: &TypeContext) -> SmeltType {
    // 1. Nested ternary: recurse to get the concrete result type.
    if let Some(nested) = extract_nested_ternary(expr) {
        let nested_result = infer_ternary_type(&nested, ctx);
        // If the nested ternary has errors (e.g. its own BranchTypeMismatch),
        // its result type is Unknown, and the outer LUB silently absorbs it:
        //  - One branch Unknown, the other concrete: the Unknown-propagation
        //    arm of `compute_ternary_lub` (lines 291–293) returns the
        //    concrete branch's type.
        //  - Both branches Unknown (e.g. both nested ternaries are
        //    ill-typed): the identical-type fast path returns `Unknown`
        //    without entering the propagation arm.
        // In both shapes the outer ternary emits no `BranchTypeMismatch`.
        // This is intentional — when the inner ternary is ill-typed, the
        // inner's own walk of `check_ternary_expr_diagnostics` emits the
        // inner diagnostic; the outer ternary should not independently flag
        // a mismatch caused by the inner's error.
        return nested_result.ty;
    }

    // 2. Meta-world inference (pipes, HOF, lists, records).
    let as_smelt = super::multi_model::infer_expression_smelt_type(expr, ctx, None);
    if as_smelt != SmeltType::Unknown {
        return as_smelt;
    }

    // 3. Scalar SQL inference.
    match infer_expression_type(expr, ctx) {
        Some(tc) => SmeltType::Expr(TypeConstraint::Concrete(tc.data_type)),
        None => SmeltType::Unknown,
    }
}

/// Extract a `TernaryExpr` from an `Expr` node, if the expression wraps a
/// `TERNARY_EXPR`.
///
/// The parser wraps each slot inside a `TERNARY_EXPR` with an `EXPRESSION` node.
/// When that slot is itself a nested ternary (`else if …`), the CST looks like:
///
/// ```text
/// EXPRESSION  ← the Expr returned by then_branch() / else_branch()
///   TERNARY_EXPR  ← the inner ternary
///     …
/// ```
///
/// This helper handles both the case where `expr.syntax()` IS a `TERNARY_EXPR`
/// directly (when `Expr::cast` accepted a bare `TERNARY_EXPR` node) and the case
/// where it is an `EXPRESSION` whose first child is a `TERNARY_EXPR`.
fn extract_nested_ternary(expr: &smelt_parser::ast::Expr) -> Option<TernaryExpr> {
    use smelt_parser::SyntaxKind::TERNARY_EXPR;

    let node = expr.syntax();

    // Case 1: the Expr itself IS a TERNARY_EXPR node.
    if node.kind() == TERNARY_EXPR {
        return TernaryExpr::cast(node.clone());
    }

    // Case 2: EXPRESSION whose first TERNARY_EXPR child is the nested ternary.
    node.children()
        .find(|c| c.kind() == TERNARY_EXPR)
        .and_then(TernaryExpr::cast)
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

/// Walk the raw file syntax node and detect bare `THEN_KW` tokens that appear
/// outside any `TERNARY_EXPR` or SQL `WHEN_CLAUSE` — these are `TernaryDanglingThen`.
///
/// This function must walk the FULL file (not just the `SelectStmt`) because the
/// parser's error recovery may eject `THEN_KW` tokens to the top-level FILE node
/// when they appear in an unexpected expression position.
///
/// Pure function — no Salsa dependency.
pub fn check_dangling_ternary_keywords(
    file_syntax: &smelt_parser::syntax_kind::SyntaxNode,
) -> Vec<crate::Diagnostic> {
    use smelt_parser::SyntaxKind::{TERNARY_EXPR, THEN_KW, WHEN_CLAUSE};

    let mut diags: Vec<crate::Diagnostic> = Vec::new();

    // Walk all tokens in the full file to find dangling THEN_KW.
    for elem in file_syntax.descendants_with_tokens() {
        let token = match elem.into_token() {
            Some(t) => t,
            None => continue,
        };
        if token.kind() != THEN_KW {
            continue;
        }
        // Check that no ancestor is a TERNARY_EXPR or WHEN_CLAUSE.
        let has_ternary_or_when_ancestor = token
            .parent_ancestors()
            .any(|a| matches!(a.kind(), TERNARY_EXPR | WHEN_CLAUSE));
        if !has_ternary_or_when_ancestor {
            diags.push(crate::Diagnostic {
                severity: crate::DiagnosticSeverity::Error,
                message: crate::meta_hof_diagnostic_message(
                    crate::DiagnosticCode::TernaryDanglingThen,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                ),
                range: token.text_range(),
                code: Some(crate::DiagnosticCode::TernaryDanglingThen),
                data: None,
            });
        }
    }

    diags
}

/// Walk a `SelectStmt` (or any syntax subtree) and emit Phase F ternary diagnostics:
///
/// - `TernaryConditionNotBoolean`: condition is not Boolean or Unknown.
/// - `TernaryBranchTypeMismatch`: then/else branches have incompatible types.
/// - `TernaryDanglingElse`: a `TERNARY_EXPR` node with no else branch.
///
/// Note: `TernaryDanglingThen` (bare `then` keyword outside a ternary expression) is
/// detected by [`check_dangling_ternary_keywords`] which walks the full file syntax.
/// This function only walks the `SelectStmt` subtree and cannot reach FILE-level tokens
/// that the parser may have ejected during error recovery.
///
/// Pure function — no Salsa dependency.
pub fn check_ternary_expr_diagnostics(
    select_stmt: &smelt_parser::ast::SelectStmt,
    ctx: &TypeContext,
) -> Vec<crate::Diagnostic> {
    use smelt_parser::SyntaxKind::TERNARY_EXPR;
    use smelt_types::format_smelt_type_hover;

    let mut diags: Vec<crate::Diagnostic> = Vec::new();

    let root = select_stmt.syntax();

    // ── Walk TERNARY_EXPR nodes ───────────────────────────────────────────────
    for node in root.descendants() {
        if node.kind() == TERNARY_EXPR {
            let ternary = TernaryExpr::cast(node.clone()).unwrap();

            // Check for dangling else (TERNARY_EXPR with no else-branch expression).
            if ternary.else_branch().is_none() {
                // Only emit if the then-branch exists (otherwise the expression is
                // severely malformed — let parse errors handle it).
                if ternary.then_branch().is_some() {
                    // Anchor at the TERNARY_EXPR node span.
                    diags.push(crate::Diagnostic {
                        severity: crate::DiagnosticSeverity::Error,
                        message: crate::meta_hof_diagnostic_message(
                            crate::DiagnosticCode::TernaryDanglingElse,
                            None,
                            None,
                            None,
                            None,
                            None,
                            None,
                            None,
                        ),
                        range: node.text_range(),
                        code: Some(crate::DiagnosticCode::TernaryDanglingElse),
                        data: None,
                    });
                }
                // Skip full inference — some children are missing.
                continue;
            }

            // Full ternary — run type inference and convert sentinels to diagnostics.
            let result = infer_ternary_type(&ternary, ctx);
            for sentinel in &result.sentinels {
                match sentinel {
                    TernarySentinel::ConditionNotBoolean { found } => {
                        // Anchor at the condition expression span.
                        let cond_range = ternary
                            .condition()
                            .map(|e| e.text_range())
                            .unwrap_or_else(|| node.text_range());
                        let found_str = format_smelt_type_hover(found);
                        diags.push(crate::Diagnostic {
                            severity: crate::DiagnosticSeverity::Error,
                            message: crate::meta_hof_diagnostic_message(
                                crate::DiagnosticCode::TernaryConditionNotBoolean,
                                None,
                                None,
                                None,
                                Some(&found_str),
                                None,
                                None,
                                None,
                            ),
                            range: cond_range,
                            code: Some(crate::DiagnosticCode::TernaryConditionNotBoolean),
                            data: None,
                        });
                    }
                    TernarySentinel::BranchTypeMismatch {
                        then_type,
                        else_type,
                    } => {
                        // Anchor at the `else` keyword (the spec says "anchored at
                        // the `else` keyword's range").
                        let else_kw_range = {
                            use smelt_parser::SyntaxKind::ELSE_KW;
                            node.children_with_tokens()
                                .filter_map(|e| e.into_token())
                                .find(|t| t.kind() == ELSE_KW)
                                .map(|t| t.text_range())
                                .unwrap_or_else(|| node.text_range())
                        };
                        let then_str = format_smelt_type_hover(then_type);
                        let else_str = format_smelt_type_hover(else_type);
                        diags.push(crate::Diagnostic {
                            severity: crate::DiagnosticSeverity::Error,
                            message: crate::meta_hof_diagnostic_message(
                                crate::DiagnosticCode::TernaryBranchTypeMismatch,
                                None,
                                None,
                                None,
                                None,
                                None,
                                Some(&then_str),
                                Some(&else_str),
                            ),
                            range: else_kw_range,
                            code: Some(crate::DiagnosticCode::TernaryBranchTypeMismatch),
                            data: None,
                        });
                    }
                }
            }
        }
    }

    diags
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

    /// Debug: check what nodes appear for `SELECT then x FROM t`.
    #[test]
    #[ignore = "diagnostic debug helper"]
    fn debug_dangling_then_cst() {
        use smelt_parser::ast::File;
        use smelt_parser::SyntaxKind::{TERNARY_EXPR, THEN_KW, WHEN_CLAUSE};
        let sql = "SELECT then x FROM t";
        let parse = smelt_parser::parse(sql);
        let root = parse.syntax();
        let file = File::cast(root.clone()).expect("FILE");
        let select_stmt = file.select_stmt();
        eprintln!("select_stmt present: {}", select_stmt.is_some());
        // Walk all tokens in the root to find THEN_KW
        for elem in root.descendants_with_tokens() {
            if let Some(token) = elem.into_token() {
                if token.kind() == THEN_KW {
                    eprintln!("THEN_KW found at {:?}", token.text_range());
                    for anc in token.parent_ancestors() {
                        eprintln!("  ancestor: {:?}", anc.kind());
                        if matches!(anc.kind(), TERNARY_EXPR | WHEN_CLAUSE) {
                            eprintln!("  -> IS ternary/when ancestor!");
                        }
                    }
                }
            }
        }
    }

    /// Regression test: a nested ternary as an else-branch must be resolved to
    /// its concrete type rather than Unknown.
    ///
    /// `if TRUE then 1 else if TRUE then 'x' else 'y'`:
    ///
    /// - outer THEN type = Integer
    /// - outer ELSE type = Text  (from the inner ternary's LUB)
    /// - `BranchTypeMismatch` must fire on the outer ternary.
    ///
    /// Previously, `infer_expression_smelt_type` had no handler for `TERNARY_EXPR`,
    /// so the else-branch resolved to `Unknown`, which the "Unknown propagates" rule
    /// silently accepted — emitting `Integer` as the result instead of an error.
    #[test]
    fn nested_ternary_else_branch_resolves_correctly() {
        use smelt_parser::ast::File;
        use smelt_parser::SyntaxKind::TERNARY_EXPR;

        // `if TRUE then 1 else if TRUE then 'x' else 'y'`
        // Outer then=Integer, outer else=Text(inner ternary) → BranchTypeMismatch
        let sql = "SELECT if TRUE then 1 else if TRUE then 'x' else 'y' FROM t";
        let parse = smelt_parser::parse(sql);
        let root = parse.syntax();
        let file = File::cast(root).expect("FILE");
        let select = file.select_stmt().expect("SelectStmt");

        // Find the outermost TERNARY_EXPR (the one with the longest span).
        let outer_node = select
            .syntax()
            .descendants()
            .filter(|n| n.kind() == TERNARY_EXPR)
            .max_by_key(|n| n.text_range().len())
            .expect("must have at least one TERNARY_EXPR");
        let outer_ternary = TernaryExpr::cast(outer_node).expect("TernaryExpr");

        let ctx = TypeContext::new();
        let result = infer_ternary_type(&outer_ternary, &ctx);

        assert!(
            result.sentinels.iter().any(|s| matches!(s, TernarySentinel::BranchTypeMismatch { .. })),
            "outer ternary must emit BranchTypeMismatch when else-branch is a type-incompatible nested ternary; \
             got sentinels: {:?}, type: {:?}",
            result.sentinels,
            result.ty,
        );
    }

    /// Also verify via check_ternary_expr_diagnostics that the diagnostic fires
    /// at the correct level when walking the full select statement.
    #[test]
    fn nested_ternary_else_branch_diagnostic_fires() {
        use crate::DiagnosticCode;
        use smelt_parser::ast::File;

        let sql = "SELECT if TRUE then 1 else if TRUE then 'x' else 'y' FROM t";
        let parse = smelt_parser::parse(sql);
        let root = parse.syntax();
        let file = File::cast(root).expect("FILE");
        let select = file.select_stmt().expect("SelectStmt");

        let ctx = TypeContext::new();
        let diags = check_ternary_expr_diagnostics(&select, &ctx);

        assert!(
            diags
                .iter()
                .any(|d| d.code == Some(DiagnosticCode::TernaryBranchTypeMismatch)),
            "check_ternary_expr_diagnostics must emit TernaryBranchTypeMismatch for \
             'if TRUE then 1 else if TRUE then 'x' else 'y''; got: {:?}",
            diags,
        );
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

    /// Lock: a ternary whose COND is a deferred (non-static) Boolean does NOT
    /// collapse the result to `Unknown` — it returns the LUB of the two branch
    /// types.  This covers the `m.has(k)` pattern: `m.has(k)` synthesises
    /// `Boolean` (D-18 / spec §"Maps" rule 4), so `if m.has(k) then m.get(k)
    /// else default` short-circuits normally without spurious `MapGetMissingKey`
    /// on the unreached branch.
    #[test]
    fn deferred_has_cond_short_circuits() {
        // `a = b` is a runtime Boolean comparison (not a static literal) —
        // a proxy for the deferred `m.has(k)` Boolean COND (which now correctly
        // synthesises to `Boolean`, not `Unknown`, after the D-18 record.rs fix).
        let result = run("a = b", "'x'", "'y'");
        assert!(
            !matches!(result.ty, SmeltType::Unknown),
            "Boolean COND must NOT collapse ternary to Unknown; got ty={:?}",
            result.ty
        );
        // Both branches are Text literals → LUB = Text.
        assert!(
            matches!(
                result.ty,
                SmeltType::Expr(TypeConstraint::Concrete(DataType::Text))
            ),
            "LUB of two Text branches must be Text; got: {:?}",
            result.ty
        );
        assert!(
            result.sentinels.is_empty(),
            "no sentinels expected for valid Boolean COND with matching branches; got: {:?}",
            result.sentinels
        );
        // Deferred (non-static) Boolean → short_circuit is None.
        assert_eq!(
            result.short_circuit, None,
            "non-static Boolean COND must produce no short-circuit hint; got: {:?}",
            result.short_circuit,
        );
    }
}
