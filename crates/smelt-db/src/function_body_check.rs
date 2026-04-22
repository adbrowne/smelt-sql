//! Phase 5 (smelt-functions) — pure body type-check with parameter binding.
//!
//! Given a parsed [`FunctionSig`] and the AST of its body, produce the body
//! diagnostics for:
//!   - `DuplicateParameterName` (anchored at the second occurrence's name span)
//!   - `UnknownIdentifier` (identifier not a param, not resolvable in any
//!     enclosing scope — inside a `smelt.define` body there is no FROM scope
//!     yet, so effectively "not a param")
//!   - `FunctionBodyTypeMismatch` (body contains a type error at an inner
//!     subexpression)
//!
//! Pure-function rule (CLAUDE.md): this module does not touch Salsa. Callers
//! in `smelt-db/src/lib.rs` (the `file_function_body_diagnostics` tracked
//! query) are responsible for building the inputs and wiring the output into
//! the diagnostic accumulator.

use rowan::TextRange;
use smelt_parser::ast::{BinaryExpr, Expr};
use smelt_parser::offset_to_position;
use smelt_types::signatures::{FunctionSig, ParamSpec, SmeltType, TypeConstraint};
use smelt_types::{DataType, TypedColumn};

use crate::type_inference::{infer_expression_type, TypeContext};
use crate::{Diagnostic, DiagnosticCode, DiagnosticSeverity, Range};

/// Check a single `smelt.define` body, producing Phase-5 diagnostics.
///
/// Arguments:
/// - `sig`: the already-extracted function signature (with parsed `type_ref`
///   payloads where available).
/// - `body`: the body expression AST (the child of `DEFINE_BODY`).
/// - `text`: the source text (post frontmatter strip) — required to convert
///   Rowan `TextRange`s into line/column [`Range`]s.
///
/// Returns diagnostics in a deterministic order. The `Diagnostic.data` field
/// is always `None` for Phase-5 diagnostics — Phase 6 adds
/// `DiagnosticData::ExpansionFrames`. Emitting `None` avoids stamping an
/// empty frame-stack on every body diagnostic.
pub fn check_function_body(sig: &FunctionSig, body: &Expr, text: &str) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    // 1. Duplicate parameter names — anchor at the second occurrence's name
    //    span. If duplicates are found we skip the rest of the checks: the
    //    signature is malformed and any downstream type-check diagnostic
    //    would be noise.
    if emit_duplicate_param_diagnostics(&sig.params, &mut diagnostics) {
        return diagnostics;
    }

    // 2. Build a TypeContext with each declared param bound to its declared
    //    type. Unparseable or absent annotations bind to `Unknown` — the
    //    Phase 4 `InvalidFunctionTypeRef` diagnostic already flagged them.
    let ctx = seed_param_context(&sig.params);

    // 3. Walk the body recursively, emitting:
    //    - `UnknownIdentifier` for bare identifiers that don't resolve.
    //    - `FunctionBodyTypeMismatch` for type-incompatible subexpressions.
    walk_body(body, &ctx, text, &mut diagnostics);

    diagnostics
}

/// Emit a `DuplicateParameterName` at each second+ occurrence of a repeated
/// parameter name. Returns `true` if any duplicates were emitted.
fn emit_duplicate_param_diagnostics(params: &[ParamSpec], out: &mut Vec<Diagnostic>) -> bool {
    use std::collections::HashSet;
    let mut seen: HashSet<&str> = HashSet::new();
    let mut any = false;
    for p in params {
        if p.name.is_empty() {
            continue;
        }
        if seen.contains(p.name.as_str()) {
            if let Some(range) = p.name_range {
                out.push(Diagnostic {
                    severity: DiagnosticSeverity::Error,
                    message: format!("Duplicate parameter name `{}`", p.name),
                    range,
                    code: Some(DiagnosticCode::DuplicateParameterName),
                    data: None,
                });
                any = true;
            }
        } else {
            seen.insert(p.name.as_str());
        }
    }
    any
}

/// Build a `TypeContext` seeded with the signature's parameter bindings.
fn seed_param_context(params: &[ParamSpec]) -> TypeContext {
    let mut ctx = TypeContext::new();
    for p in params {
        if p.name.is_empty() {
            continue;
        }
        let bound_type = param_binding_type(p);
        ctx.add_function_param(&p.name, TypedColumn::nullable(bound_type));
    }
    ctx
}

/// Decide the concrete [`DataType`] to bind a parameter to inside the body
/// context.
///
/// - `Expr<Concrete(T)>` → `T`.
/// - `Expr<Numeric>` → `DataType::Double` — the widest numeric type per §16 #9.
///   This is a conservative stand-in: any valid numeric expression built from
///   the param will type-check, and all numeric operators accept Double.
/// - `Expr<Any>` → `DataType::Unknown` — "no constraint" means we intentionally
///   decline to type-check through the param.
/// - Unparsed / unannotated / malformed → `DataType::Unknown`. Phase 4 already
///   surfaced the parse error; we avoid cascading into spurious body errors.
fn param_binding_type(p: &ParamSpec) -> DataType {
    match &p.type_ref {
        Some(Ok(SmeltType::Expr(TypeConstraint::Concrete(dt)))) => dt.clone(),
        Some(Ok(SmeltType::Expr(TypeConstraint::Numeric))) => DataType::Double,
        Some(Ok(SmeltType::Expr(TypeConstraint::Any))) => DataType::Unknown,
        Some(Err(_)) => DataType::Unknown,
        None => DataType::Unknown,
    }
}

/// Convert a Rowan [`TextRange`] to a line/column [`Range`] against `text`.
fn to_range(tr: TextRange, text: &str) -> Range {
    Range {
        start: offset_to_position(text, usize::from(tr.start())),
        end: offset_to_position(text, usize::from(tr.end())),
    }
}

/// Recursive body walker. Emits diagnostics for unknown identifiers and
/// binary-expression type mismatches. Returns the inferred type of the
/// sub-expression (so parents can detect mismatches), or `None` if no type
/// could be inferred.
fn walk_body(
    expr: &Expr,
    ctx: &TypeContext,
    text: &str,
    out: &mut Vec<Diagnostic>,
) -> Option<TypedColumn> {
    // Binary expression: recurse into each operand, then check the operator's
    // type-compatibility constraint. The mismatch is anchored at the binary
    // node itself — the smallest subexpression that exhibits the error.
    if let Some(binary) = expr.as_binary() {
        return walk_binary(expr, &binary, ctx, text, out);
    }

    // Bare column reference: if the identifier doesn't resolve through the
    // function-param-aware lookup, emit `UnknownIdentifier`. Qualified
    // references (e.g. `t.x`) are not checked here in Step 1 — they cannot
    // arrive from params and we don't have a FROM scope yet.
    if !has_expr_children(expr) {
        if let Some(col_ref) = expr.as_column_ref() {
            if col_ref.qualifier().is_none()
                && ctx.lookup_identifier(None, col_ref.name()).is_none()
            {
                // The lookup might have still resolved as a literal keyword
                // (NULL, TRUE, FALSE) or a typed literal — defer to
                // `infer_expression_type` to confirm. If inference returns
                // None, it's genuinely an unknown identifier.
                if infer_expression_type(expr, ctx).is_none() {
                    out.push(Diagnostic {
                        severity: DiagnosticSeverity::Error,
                        message: format!(
                            "Unknown identifier `{}` — not a parameter or in any enclosing scope",
                            col_ref.name()
                        ),
                        range: to_range(expr.text_range(), text),
                        code: Some(DiagnosticCode::UnknownIdentifier),
                        data: None,
                    });
                    return None;
                }
            }
        }
    }

    // For every other expression shape (CAST, CASE, function call, etc.)
    // recurse into all `Expr` children so we find nested mismatches and
    // unknown identifiers. We intentionally do not walk subqueries — those
    // are out of scope for a `smelt.define` body in Step 1.
    if expr.as_subquery().is_none() && expr.as_exists().is_none() {
        for child in expr.syntax().children() {
            if let Some(child_expr) = Expr::cast(child) {
                walk_body(&child_expr, ctx, text, out);
            }
        }
    }

    // Fall back to the existing type-inference engine for this subexpression's
    // type. Empty function_params means this is identical to prior behaviour
    // for non-body callers.
    infer_expression_type(expr, ctx)
}

/// Walk a binary expression, emitting a type-mismatch diagnostic when the
/// operator's domain can't be satisfied by the resolved operand types.
fn walk_binary(
    expr: &Expr,
    binary: &BinaryExpr,
    ctx: &TypeContext,
    text: &str,
    out: &mut Vec<Diagnostic>,
) -> Option<TypedColumn> {
    let before_len = out.len();

    // Recurse into operands first so inner diagnostics surface.
    let child_exprs: Vec<Expr> = binary.node().children().filter_map(Expr::cast).collect();
    let mut operand_types = Vec::with_capacity(child_exprs.len());
    for child in &child_exprs {
        operand_types.push(walk_body(child, ctx, text, out));
    }

    // If recursion already emitted a mismatch/unknown *inside* the operands,
    // don't stack a second mismatch for this outer node — the inner one is
    // the tighter anchor the review checklist asks for. We recognise
    // "already reported" by the out-vec length delta.
    let inner_reported = out.len() > before_len;

    // Decide whether the operator's type domain is violated.
    let Some(op) = binary.operator() else {
        return infer_expression_type(expr, ctx);
    };
    let op_upper = op.to_uppercase();

    // Only operators with a strict numeric domain get a Phase-5 mismatch.
    // Logical / comparison / pattern-matching operators are permissive in
    // SQL (compare anything, coerce to bool). Arithmetic is where type
    // errors bite in practice — §2 of the research models the canonical
    // `x + 'text'` failure against Integer+Text. Keep the domain tight so
    // we don't flag legal expressions.
    let arithmetic = matches!(op_upper.as_str(), "+" | "-" | "*" | "/" | "%");

    if arithmetic && !inner_reported && child_exprs.len() == 2 {
        let left = operand_types[0].as_ref();
        let right = operand_types[1].as_ref();
        if let (Some(l), Some(r)) = (left, right) {
            if !is_arithmetic_compatible(&l.data_type, &r.data_type, &op_upper) {
                out.push(Diagnostic {
                    severity: DiagnosticSeverity::Error,
                    message: format!(
                        "Cannot apply `{}` to `{}` and `{}` in function body",
                        op, l.data_type, r.data_type
                    ),
                    range: to_range(expr.text_range(), text),
                    code: Some(DiagnosticCode::FunctionBodyTypeMismatch),
                    data: None,
                });
                return None;
            }
        }
    }

    infer_expression_type(expr, ctx)
}

/// Can `op` be applied to `left` and `right` without producing a type error
/// per Phase-5's conservative Tier-1 rules?
///
/// Tier-1 rule of thumb: both operands must be numeric, or the pair must be
/// one of the well-known temporal/interval combinations already recognised by
/// `infer_binary_expr_type`. Unknown types pass (we can't prove a violation).
fn is_arithmetic_compatible(left: &DataType, right: &DataType, op: &str) -> bool {
    use DataType::*;

    // Pass-through for Unknown — we simply don't know.
    if matches!(left, Unknown | Null) || matches!(right, Unknown | Null) {
        return true;
    }

    let both_numeric = left.is_numeric() && right.is_numeric();
    if both_numeric {
        return true;
    }

    // Temporal arithmetic shapes recognised by the existing type-inference
    // engine. We only need to call these out for `+` and `-` — `*`, `/`,
    // `%` have a numeric-only domain here (interval * numeric is handled by
    // the numeric branch since numeric side counts).
    match op {
        "+" | "-" => match (left, right) {
            (Date, Interval) | (Interval, Date) => true,
            (Timestamp { .. }, Interval) | (Interval, Timestamp { .. }) => true,
            (Time, Interval) | (Interval, Time) => true,
            (Interval, Interval) => true,
            (Date, Date) if op == "-" => true,
            (Timestamp { .. }, Timestamp { .. }) if op == "-" => true,
            (Time, Time) if op == "-" => true,
            _ => false,
        },
        "*" | "/" => matches!((left, right), (Interval, _) | (_, Interval)),
        _ => false,
    }
}

/// Does `expr` have any child `Expr` nodes? Mirrors the logic in
/// `walk_expression_columns_with_visitor` for the leaf-detection heuristic.
fn has_expr_children(expr: &Expr) -> bool {
    expr.syntax().children().any(|c| Expr::cast(c).is_some())
}

#[cfg(test)]
mod tests {
    use super::*;
    use smelt_parser::ast::{File as AstFile, SmeltDefine};
    use smelt_parser::{parse, strip_frontmatter};
    use smelt_types::signatures::extract_function_signatures;

    fn parse_define(text: &str) -> (FunctionSig, Expr, String) {
        let clean = strip_frontmatter(text).to_string();
        let p = parse(&clean);
        let ast = AstFile::cast(p.syntax()).expect("FILE");
        let sig = extract_function_signatures(&ast, &clean)
            .into_iter()
            .next()
            .expect("one define");
        let define: SmeltDefine = ast.defines().next().expect("one SmeltDefine");
        let body_expr = define
            .body()
            .and_then(|b| b.expression())
            .expect("body has expression");
        (sig, body_expr, clean)
    }

    #[test]
    fn check_body_ok_for_simple_numeric_add() {
        let (sig, body, text) =
            parse_define("smelt.define f(x: Expr<Integer>, y: Expr<Integer>) AS (x + y)\n");
        let diags = check_function_body(&sig, &body, &text);
        assert!(diags.is_empty(), "expected no diagnostics, got {diags:?}");
    }

    #[test]
    fn check_body_detects_integer_plus_text() {
        let (sig, body, text) =
            parse_define("smelt.define bad(x: Expr<Integer>) AS (x + 'text')\n");
        let diags = check_function_body(&sig, &body, &text);
        assert_eq!(diags.len(), 1, "expected one diagnostic: {diags:?}");
        assert_eq!(
            diags[0].code,
            Some(DiagnosticCode::FunctionBodyTypeMismatch)
        );
    }

    #[test]
    fn check_body_detects_unknown_identifier() {
        let (sig, body, text) = parse_define("smelt.define bad(x: Expr<Integer>) AS (z)\n");
        let diags = check_function_body(&sig, &body, &text);
        assert_eq!(diags.len(), 1, "expected one diagnostic: {diags:?}");
        assert_eq!(diags[0].code, Some(DiagnosticCode::UnknownIdentifier));
    }

    #[test]
    fn check_body_short_circuits_on_duplicate_params() {
        let (sig, body, text) =
            parse_define("smelt.define f(x: Expr<Integer>, x: Expr<Integer>) AS (x + z)\n");
        let diags = check_function_body(&sig, &body, &text);
        // Only the duplicate-param diagnostic — no unknown-ident for `z`
        // because we bail early on duplicates.
        assert_eq!(diags.len(), 1, "expected only dup diag: {diags:?}");
        assert_eq!(diags[0].code, Some(DiagnosticCode::DuplicateParameterName));
    }

    #[test]
    fn check_body_numeric_param_accepts_double_division() {
        // Expr<Numeric> binds to Double, so CAST(x AS DOUBLE) / y is clean.
        let (sig, body, text) = parse_define(
            "smelt.define safe_divide(numerator: Expr<Numeric>, denominator: Expr<Numeric>) \
             -> Expr<Double> AS (CAST(numerator AS DOUBLE) / denominator)\n",
        );
        let diags = check_function_body(&sig, &body, &text);
        assert!(diags.is_empty(), "expected no diagnostics: {diags:?}");
    }
}
