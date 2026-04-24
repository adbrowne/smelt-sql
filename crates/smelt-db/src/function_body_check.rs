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
use smelt_parser::ast::{BinaryExpr, Cte, Expr, SelectStmt, SmeltFnCall};
use smelt_parser::offset_to_position;
use smelt_types::signatures::{
    check_schema_requirement, unify_call, ContextRef, ExprKind, FrameInfo, FunctionSig, ParamSpec,
    SchemaMismatch, SchemaRequirement, Signature, SmeltType, Tier, TypeConstraint,
    UnificationError,
};
use smelt_types::{DataType, TypedColumn};
use std::path::PathBuf;

use crate::schema::{Column, ColumnSource, ModelSchema};
use crate::type_inference::{
    check_undeclared_columns, infer_cte_columns, infer_expression_kind, infer_expression_type,
    walk_expression_columns_with_visitor, walk_select_columns_with_visitor, TypeContext,
};
use crate::{Diagnostic, DiagnosticCode, DiagnosticData, DiagnosticSeverity, Range};

/// Discriminates the three type-checking tiers (§8) inside
/// `check_function_body` and `check_smelt_fn_call`.
///
/// - `Tier1Expansion`: call-site expansion; `arg_types` holds the
///   concrete argument types bound from the caller. Used in Phase 26.
/// - `Tier2Isolated`: definition-time check; declared parameter types
///   seed the body context. Used by `function_body_diagnostics_for_file`
///   for Tier 2 functions.
/// - `Tier2CallSite`: pre-expansion call-site check against declared
///   types; `expected_ret` is the type context expects the call to
///   return (for bidirectional inference, Phase 25+).
#[derive(Debug, Clone)]
pub enum CheckMode {
    Tier1Expansion(Vec<(String, DataType)>),
    Tier2Isolated,
    Tier2CallSite(Option<DataType>),
}

/// Return `true` when every non-`TableExpr` / non-`SelectItems` parameter
/// in `sig` has an explicit, parseable `Expr<T>` annotation.  Exactly the
/// condition that enables definition-time body checking in isolation (Tier 2).
pub fn is_tier2_function(sig: &FunctionSig) -> bool {
    sig.params.iter().all(|p| match &p.type_ref {
        Some(Ok(SmeltType::TableExpr(_))) => true, // TableExpr params are exempt
        Some(Ok(SmeltType::SelectItems { .. })) => true, // SelectItems are exempt
        Some(Ok(SmeltType::Expr(_))) => true,      // annotated scalar — counts
        _ => false,                                // unannotated or malformed → Tier 1
    })
}

/// Shape of a function body returned by `body_lookup`.
///
/// Phase 15 introduces the `Select` variant for `TableExpr`-returning
/// defines whose body is a bare top-level `(SELECT ... FROM source)`.
/// The walker paths are different — SELECT bodies use
/// [`check_undeclared_columns`] against the FROM-seeded `TypeContext`;
/// Expression bodies use the Phase 5 `walk_body` recursion.
#[derive(Debug, Clone)]
pub enum BodyShape {
    /// Parenthesised scalar expression body (Phase 5).
    Expression(Expr),
    /// Top-level SELECT body (Phase 15+). Introduced by `TableExpr`-
    /// returning `smelt.define`s whose body is a SELECT rather than a
    /// scalar expression.
    Select(SelectStmt),
}

/// Callback type for dispatching nested `smelt.fn.*` calls encountered
/// during body-recursion (Phase 12).
///
/// When `check_smelt_fn_call` re-walks a body with the call-site-derived
/// bindings, the walker encounters further nested `smelt.fn.*` calls.
/// This closure lets us recursively invoke the same checker so frames
/// stack up across arbitrary expansion depth. In unit tests and the
/// legacy `check_function_body` entrypoint this is `None` — body walks
/// that never expand further work exactly as they did in Phase 6.
pub type NestedCallHandler<'a> = dyn Fn(&SmeltFnCall, &TypeContext, &str) -> Vec<Diagnostic> + 'a;

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
    check_function_body_inner(sig, body, text, None)
}

/// Phase-12 variant of [`check_function_body`] that dispatches nested
/// `smelt.fn.*` calls through `nested` so frames stack up across
/// arbitrary expansion depth.
///
/// Callers pass a closure that invokes [`check_smelt_fn_call`] on each
/// nested call with the re-bound context. The plain
/// [`check_function_body`] entrypoint delegates here with `None` and
/// therefore matches its pre-Phase-12 behaviour exactly.
pub fn check_function_body_with_expansion(
    sig: &FunctionSig,
    body: &Expr,
    text: &str,
    nested: &NestedCallHandler<'_>,
) -> Vec<Diagnostic> {
    check_function_body_inner(sig, body, text, Some(nested))
}

/// Phase-26 variant: walk a function body using a *caller-supplied* `TypeContext`
/// rather than re-building one from the signature's parameter annotations.
///
/// Used by [`check_smelt_fn_call`] when expanding a Tier 1 body at a call site —
/// the call-site-bound `body_ctx` already has concrete argument types, so we skip
/// `seed_param_context` and use it directly. Duplicate-parameter checks are also
/// skipped (they already ran at definition time via `function_body_diagnostics_for_file`).
pub fn walk_body_with_ctx(
    body: &Expr,
    ctx: &TypeContext,
    text: &str,
    nested: &NestedCallHandler<'_>,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    walk_body(body, ctx, text, &mut diagnostics, Some(nested));
    diagnostics
}

fn check_function_body_inner(
    sig: &FunctionSig,
    body: &Expr,
    text: &str,
    nested: Option<&NestedCallHandler<'_>>,
) -> Vec<Diagnostic> {
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
    walk_body(body, &ctx, text, &mut diagnostics, nested);

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
        // `Ordered` (Phase 7) currently has no precise stand-in type —
        // treat it like `Any` in the body checker until Phase 8 adds the
        // generics plumbing that actually binds `T` from call-site args.
        Some(Ok(SmeltType::Expr(TypeConstraint::Ordered))) => DataType::Unknown,
        Some(Ok(SmeltType::Expr(TypeConstraint::Any))) => DataType::Unknown,
        // `TableExpr` (Phase 15) params are bound as FROM-scope entries,
        // not as `function_params`. When reached here (e.g. by the
        // unified Tier-1 body seeder) we fall back to `Unknown`.
        Some(Ok(SmeltType::TableExpr(_))) => DataType::Unknown,
        // `SelectItems<Kind>` (Phase 21) params are list-typed; fall back to Unknown.
        Some(Ok(SmeltType::SelectItems { .. })) => DataType::Unknown,
        Some(Err(_)) => DataType::Unknown,
        None => DataType::Unknown,
    }
}

/// Is this parameter a `TableExpr` sort?
///
/// Phase 15 treats `TableExpr` parameters differently from `Expr<T>`
/// ones: they contribute their caller-supplied schema to the body's
/// FROM-scope rather than binding a single typed column under the
/// parameter's name.
pub(crate) fn is_tableexpr_param(p: &ParamSpec) -> bool {
    matches!(&p.type_ref, Some(Ok(SmeltType::TableExpr(_))))
}

/// Return the [`SchemaRequirement`] declared on a `TableExpr<{…}>`
/// parameter, if any (Phase 16).
///
/// Returns `None` for bare `TableExpr` parameters and for non-
/// `TableExpr` parameters. Callers that already know the parameter is
/// a `TableExpr` via [`is_tableexpr_param`] can treat a returned
/// `None` as "no row requirement to check".
pub(crate) fn tableexpr_schema_requirement(p: &ParamSpec) -> Option<&SchemaRequirement> {
    match &p.type_ref {
        Some(Ok(SmeltType::TableExpr(req))) => req.as_ref(),
        _ => None,
    }
}

/// Build a [`Diagnostic`] describing a row-requirement failure
/// (Phase 16).
///
/// Anchored at the argument expression's span. Message shape mirrors
/// the other call-site diagnostics (`ArgTypeMismatch`,
/// `MissingArgument`) so users see consistent wording. The structured
/// [`SchemaMismatch`] variant drives the message — missing column,
/// type mismatch, or unexpected extras.
fn row_requirement_diagnostic(
    mismatch: &SchemaMismatch,
    fn_name: &str,
    param_name: &str,
    arg_range: Range,
) -> Diagnostic {
    let message = match mismatch {
        SchemaMismatch::MissingColumn { column, required } => format!(
            "Argument for parameter `{}` of `smelt.fn.{}` is missing required column `{}: {}`",
            param_name,
            fn_name,
            column,
            required.render()
        ),
        SchemaMismatch::TypeMismatch {
            column,
            required,
            actual,
        } => format!(
            "Column `{}` in argument for parameter `{}` of `smelt.fn.{}` has type `{}`, expected `{}`",
            column,
            param_name,
            fn_name,
            actual,
            required.render()
        ),
    };
    Diagnostic {
        severity: DiagnosticSeverity::Error,
        message,
        range: arg_range,
        code: Some(DiagnosticCode::RowRequirementUnsatisfied),
        data: None,
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
///
/// `nested` is the Phase-12 hook for recursively dispatching nested
/// `smelt.fn.*` calls through [`check_smelt_fn_call`]. When `Some`, every
/// nested `SMELT_FN_CALL` encountered in the body is checked via the
/// closure so frames stack up across arbitrary expansion depth. When
/// `None` the walker matches its pre-Phase-12 behaviour — useful for
/// unit tests and the [`check_function_body`] legacy entry.
fn walk_body(
    expr: &Expr,
    ctx: &TypeContext,
    text: &str,
    out: &mut Vec<Diagnostic>,
    nested: Option<&NestedCallHandler<'_>>,
) -> Option<TypedColumn> {
    // Phase 12: nested `smelt.fn.*` call — dispatch through the handler so
    // the call-site checker recurses with the caller's bindings visible.
    // The handler emits any call-site diagnostics (and body-cascade
    // diagnostics with `ExpansionFrames`). We still fall through to
    // generic inference for the return type below.
    if let (Some(call), Some(nested)) = (expr.as_smelt_fn_call(), nested) {
        let nested_diags = nested(&call, ctx, text);
        out.extend(nested_diags);
        // Still compute a return type via the inference engine — when the
        // call checks clean, the inferred type flows up to the parent
        // expression (e.g. a binary-op) for further type checking.
        return infer_expression_type(expr, ctx);
    }

    // Binary expression: recurse into each operand, then check the operator's
    // type-compatibility constraint. The mismatch is anchored at the binary
    // node itself — the smallest subexpression that exhibits the error.
    if let Some(binary) = expr.as_binary() {
        return walk_binary(expr, &binary, ctx, text, out, nested);
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
                walk_body(&child_expr, ctx, text, out, nested);
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
    nested: Option<&NestedCallHandler<'_>>,
) -> Option<TypedColumn> {
    let before_len = out.len();

    // Recurse into operands first so inner diagnostics surface.
    let child_exprs: Vec<Expr> = binary.node().children().filter_map(Expr::cast).collect();
    let mut operand_types = Vec::with_capacity(child_exprs.len());
    for child in &child_exprs {
        operand_types.push(walk_body(child, ctx, text, out, nested));
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

/// Check a single `smelt.fn.<path>(args)` call site.
///
/// Arguments:
/// - `call`: the parsed `SMELT_FN_CALL` node.
/// - `ctx`: the call-site [`TypeContext`] — used to infer the types of
///   positional / named argument expressions. Must not be a body-context
///   (the caller composes this during expansion).
/// - `text`: the stripped source text of the *file containing the call*,
///   needed for span → line/column conversion.
/// - `sig_lookup`: resolves a bare function name to its
///   [`FunctionSig`]. Pure dependency — in Salsa it is a thin closure over
///   `resolve_function`; in unit tests it is an in-memory `HashMap` lookup.
///   Covers user-declared `smelt.define` and `smelt.extern` functions.
/// - `builtin_lookup`: resolves a bare function name to its
///   [`Signature`] in the built-in registry. Pure dependency — in Salsa
///   it is a thin closure over `smelt_types::BuiltinRegistry::resolve`.
///   Consulted when `sig_lookup` misses so the checker dispatches through
///   `unify_call` for built-ins (Phase 10 unified-resolver path).
/// - `lub`: numeric least-upper-bound adapter for `unify_call` (§16 #14);
///   shaped to match the signature of `smelt_types::signatures::unify_call`.
///   Only used on the built-in branch.
/// - `body_lookup`: given a resolved function, produces the stripped source
///   text and the body [`Expr`] so the checker can re-walk the body with
///   the call-site-derived bindings. Pure — the closure owns any I/O.
///   Returns `None` for externs (no body) and for any signature without a
///   define body — the checker skips body re-walk in that case.
///
/// Returns a list of diagnostics:
///   - [`DiagnosticCode::UnknownSmeltFn`] at the call-path span.
///   - [`DiagnosticCode::MissingArgument`] at the call-path span.
///   - [`DiagnosticCode::ArgTypeMismatch`] at the offending argument's span.
///   - Any [`DiagnosticCode::FunctionBodyTypeMismatch`] /
///     [`DiagnosticCode::UnknownIdentifier`] surfaced by re-walking the
///     callee's body with the call-site bindings; these carry
///     `DiagnosticData::ExpansionFrames(Vec<FrameInfo>)` with the outermost
///     entry describing this call site's bindings.
///
/// Diagnostics from deeply nested calls already carry a frame stack — this
/// function appends its frame to the *end* so callers rendering innermost-only
/// (Phase 6) or full-stack (Phase 12) get a deterministic ordering.
///
/// Pure: no Salsa access. Callers in `smelt-db/src/lib.rs` build the closures.
///
/// `decl_lookup` resolves the file path that declares the given
/// [`FunctionSig`] — used to populate `FrameInfo::decl_path` so LSP
/// clients can render each frame's related-information link against
/// the correct file (Phase 12, §16 #16). Returns `None` on lookup-miss;
/// the frame then carries `decl_path: None` and the LSP falls back to
/// inline-only rendering for that frame.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn check_smelt_fn_call(
    call: &SmeltFnCall,
    ctx: &TypeContext,
    text: &str,
    sig_lookup: &dyn Fn(&str) -> Option<FunctionSig>,
    builtin_lookup: &dyn Fn(&str) -> Option<&'static Signature>,
    lub: &dyn Fn(&DataType, &DataType) -> DataType,
    body_lookup: &dyn Fn(&FunctionSig) -> Option<(String, BodyShape)>,
    decl_lookup: &dyn Fn(&FunctionSig) -> Option<PathBuf>,
    // Phase 15: resolve a `TableExpr`-argument expression to its
    // caller-supplied schema. Called once per `TableExpr` parameter at
    // call-site expansion. Returns `None` when the arg shape is not
    // resolvable (e.g. a nested expression whose schema can't be
    // inferred here) — the body check then proceeds with no seeded
    // schema for that parameter, which surfaces as `UnknownIdentifier`
    // on bare column references inside the body.
    tableexpr_schema_lookup: &dyn Fn(&Expr, &TypeContext) -> Option<Vec<(String, TypedColumn)>>,
    // Phase 17: resolve the default value of a parameter. Called when
    // the caller omits an argument for a parameter with `has_default`.
    // Returns the inferred type of the default expression, or `None`
    // if the default cannot be located / inferred.
    default_type_lookup: &dyn Fn(&FunctionSig, &str) -> Option<DataType>,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    // 1. Resolve the function. The path segments after `smelt.fn.` join with
    //    `.`; only the trailing leaf name is what the workspace signature
    //    index keys on (namespace segments are informational until Step 2's
    //    backend namespace work).
    let path_range = match call.call_path_range() {
        Some(r) => to_range(r, text),
        None => to_range(call.text_range(), text),
    };
    let segments = call.call_path().map(|p| p.segments()).unwrap_or_default();
    let Some(name) = segments.last().cloned() else {
        // Parser already flagged the missing-name error; nothing more to do.
        return diagnostics;
    };

    let Some(sig) = sig_lookup(&name) else {
        // No user-declared function — try the built-in registry.
        // On a hit, dispatch through `unify_call` which yields
        // `ArgTypeMismatch` / `MissingArgument` / arity diagnostics.
        if let Some(builtin_sig) = builtin_lookup(&name) {
            return check_builtin_call(call, &name, path_range, builtin_sig, ctx, text, lub);
        }
        diagnostics.push(Diagnostic {
            severity: DiagnosticSeverity::Error,
            message: format!("Unknown function `smelt.fn.{}`", name),
            range: path_range,
            code: Some(DiagnosticCode::UnknownSmeltFn),
            data: None,
        });
        return diagnostics;
    };

    // 2. Bind arguments to parameters. Positional first, then named.
    let arg_list = call.arg_list();
    let positional: Vec<Expr> = arg_list
        .as_ref()
        .map(|al| al.positional_args())
        .unwrap_or_default();
    let named: Vec<smelt_parser::ast::NamedParam> = arg_list
        .as_ref()
        .map(|al| al.named_params().collect())
        .unwrap_or_default();

    // Build the name → (arg Expr, span TextRange) map. A positional slot
    // populates `sig.params[i]`; a named slot looks up `sig.params` by name.
    let mut bindings: std::collections::HashMap<String, (Expr, TextRange)> =
        std::collections::HashMap::new();

    for (i, arg) in positional.iter().enumerate() {
        if let Some(p) = sig.params.get(i) {
            bindings.insert(p.name.clone(), (arg.clone(), arg.text_range()));
        }
        // Extra positional args beyond declared params — Phase 6 silently
        // ignores (Step 2+ will emit an arity diagnostic).
    }

    for np in &named {
        let Some(nm) = np.name() else { continue };
        // If there's no parseable expression on the RHS we leave the binding
        // absent — the missing-arg check will fire below. Otherwise use the
        // span of the value expression for arg-type-mismatch diagnostics so
        // the squiggle lands on the bad value, not the whole `name => value`
        // pair.
        if let Some(value_expr) = np.value_expr() {
            bindings.insert(nm.clone(), (value_expr.clone(), value_expr.text_range()));
        }
    }

    // Phase 29: PASSING clauses. Each `PASSING name AS (body)` provides an
    // alternative binding for a fragment-sort parameter. Unknown names emit
    // `UnknownPassingParameter`; known names contribute to `bindings` like any
    // other argument, augmenting or overriding positional/named bindings for
    // the same parameter (last writer wins — positional args should not supply
    // the same param as a PASSING clause, but we don't error on that here).
    let passing_clauses: Vec<smelt_parser::ast::PassingClause> = call.passing_clauses().collect();
    for clause in &passing_clauses {
        let Some(clause_name) = clause.name() else {
            continue;
        };
        // Check if this name matches a declared parameter in the signature.
        let param_exists = sig.params.iter().any(|p| p.name == clause_name);
        if !param_exists {
            let clause_range = clause
                .name_range()
                .map(|r| to_range(r, text))
                .unwrap_or(path_range);
            diagnostics.push(Diagnostic {
                severity: DiagnosticSeverity::Error,
                message: format!(
                    "PASSING clause names `{}` which is not a parameter of `smelt.fn.{}`",
                    clause_name, sig.name
                ),
                range: clause_range,
                code: Some(DiagnosticCode::UnknownPassingParameter),
                data: None,
            });
            continue;
        }
        if let Some(body_expr) = clause.body_expr() {
            let body_range = body_expr.text_range();
            bindings.insert(clause_name, (body_expr, body_range));
        }
    }

    // 3. Build call-site binding types + detect missing / type-mismatched args.
    let mut body_ctx = TypeContext::new();
    let mut frame_bindings: Vec<(String, String)> = Vec::new(); // (param, bound_type_str)

    for param in &sig.params {
        if param.name.is_empty() {
            continue;
        }

        // Phase 15/16: `TableExpr` parameters contribute the
        // caller-supplied schema as a FROM-scope entry. For
        // `TableExpr<{…}>` (Phase 16) we first run the structured
        // row-requirement check; on failure we emit
        // `RowRequirementUnsatisfied` at the argument span, skip
        // seeding the parameter's FROM-scope, and *short-circuit the
        // body check below* (cleared by the presence of at least one
        // error-severity diagnostic on `diagnostics`). On success we
        // seed the body ctx and record any named row-variable
        // binding into the per-call `row_var_env`.
        if is_tableexpr_param(param) {
            if let Some((arg_expr, arg_range)) = bindings.get(&param.name) {
                let cols = tableexpr_schema_lookup(arg_expr, ctx).unwrap_or_default();

                if let Some(req) = tableexpr_schema_requirement(param) {
                    // Convert the columns to the (name, DataType)
                    // shape expected by the pure checker.
                    let arg_schema: Vec<(String, DataType)> = cols
                        .iter()
                        .map(|(n, tc)| (n.clone(), tc.data_type.clone()))
                        .collect();
                    match check_schema_requirement(req, &arg_schema) {
                        Ok(binding) => {
                            // Success: bind the FROM-scope and record
                            // any named row-variable binding.
                            body_ctx.add_tableexpr_param(&param.name, &cols);
                            if let Some(b) = binding {
                                body_ctx.set_row_var_binding(&b.name, b.extras);
                            }
                            frame_bindings.push((param.name.clone(), "TableExpr".to_string()));
                        }
                        Err(mismatch) => {
                            // Failure: emit a RowRequirementUnsatisfied
                            // diagnostic at the argument span. The
                            // non-empty `diagnostics` then short-
                            // circuits the body re-walk below.
                            diagnostics.push(row_requirement_diagnostic(
                                &mismatch,
                                &sig.name,
                                &param.name,
                                to_range(*arg_range, text),
                            ));
                            frame_bindings
                                .push((param.name.clone(), "<row-req-failed>".to_string()));
                        }
                    }
                } else {
                    // Bare `TableExpr` — no row requirement to run.
                    body_ctx.add_tableexpr_param(&param.name, &cols);
                    frame_bindings.push((param.name.clone(), "TableExpr".to_string()));
                }
            } else if !param.has_default {
                diagnostics.push(Diagnostic {
                    severity: DiagnosticSeverity::Error,
                    message: format!(
                        "Missing required argument `{}` for `smelt.fn.{}`",
                        param.name, sig.name
                    ),
                    range: path_range,
                    code: Some(DiagnosticCode::MissingArgument),
                    data: None,
                });
                frame_bindings.push((param.name.clone(), "<missing>".to_string()));
            } else {
                frame_bindings.push((param.name.clone(), "<default>".to_string()));
            }
            continue;
        }

        match bindings.get(&param.name) {
            Some((arg_expr, arg_range)) => {
                // Infer the argument's type in the *call-site* context.
                let arg_type = infer_expression_type(arg_expr, ctx)
                    .map(|t| t.data_type)
                    .unwrap_or(DataType::Unknown);

                // Type-check against the parameter's declared constraint (if
                // any). Unknown / malformed annotations skip the check —
                // Phase 4's `InvalidFunctionTypeRef` already fired.
                let constraint_violation = match &param.type_ref {
                    Some(Ok(SmeltType::Expr(TypeConstraint::Concrete(expected)))) => {
                        !matches!(arg_type, DataType::Unknown | DataType::Null)
                            && !types_assignment_compatible(expected, &arg_type)
                    }
                    Some(Ok(SmeltType::Expr(TypeConstraint::Numeric))) => {
                        !matches!(arg_type, DataType::Unknown | DataType::Null)
                            && !arg_type.is_numeric()
                    }
                    Some(Ok(SmeltType::Expr(TypeConstraint::Any))) => false,
                    _ => false,
                };

                if constraint_violation {
                    let expected_text = match &param.type_ref {
                        Some(Ok(SmeltType::Expr(TypeConstraint::Concrete(dt)))) => dt.to_string(),
                        Some(Ok(SmeltType::Expr(TypeConstraint::Numeric))) => "Numeric".to_string(),
                        _ => "<unknown>".to_string(),
                    };
                    diagnostics.push(Diagnostic {
                        severity: DiagnosticSeverity::Error,
                        message: format!(
                            "Argument `{}` has type `{}`, which does not satisfy parameter `{}: {}` of `{}`",
                            arg_expr.text().trim(),
                            arg_type,
                            param.name,
                            expected_text,
                            sig.name
                        ),
                        range: to_range(*arg_range, text),
                        code: Some(DiagnosticCode::ArgTypeMismatch),
                        data: None,
                    });
                    // Still bind the param to Unknown so downstream body walks
                    // don't cascade into spurious errors from the bad arg.
                    body_ctx
                        .add_function_param(&param.name, TypedColumn::nullable(DataType::Unknown));
                    frame_bindings.push((param.name.clone(), arg_type.to_string()));
                    continue;
                }

                body_ctx.add_function_param(&param.name, TypedColumn::nullable(arg_type.clone()));
                frame_bindings.push((param.name.clone(), arg_type.to_string()));
            }
            None => {
                if !param.has_default {
                    diagnostics.push(Diagnostic {
                        severity: DiagnosticSeverity::Error,
                        message: format!(
                            "Missing required argument `{}` for `smelt.fn.{}`",
                            param.name, sig.name
                        ),
                        range: path_range,
                        code: Some(DiagnosticCode::MissingArgument),
                        data: None,
                    });
                    // Bind Unknown so the body walk doesn't cascade.
                    body_ctx
                        .add_function_param(&param.name, TypedColumn::nullable(DataType::Unknown));
                    frame_bindings.push((param.name.clone(), "<missing>".to_string()));
                } else {
                    // Phase 17: default value expansion runs end-to-
                    // end. Ask the default-type lookup for the
                    // expression's inferred type; if the lookup hits,
                    // bind with that type (so the body typechecks as
                    // if the user had passed the default literally).
                    // Fallback stays `Unknown` when inference fails.
                    let dt = default_type_lookup(&sig, &param.name).unwrap_or(DataType::Unknown);
                    body_ctx.add_function_param(&param.name, TypedColumn::nullable(dt.clone()));
                    // Provenance: mark the binding as synthesized so a
                    // future frame renderer can display "default
                    // applied". Reuse the frame_bindings slot —
                    // attaching a dedicated provenance field would
                    // require touching FrameInfo across Phase 12's LSP
                    // surface; Phase 17 keeps the signal in the
                    // bound-type string for now.
                    frame_bindings.push((param.name.clone(), format!("{} (default)", dt)));
                }
            }
        }
    }

    // Phase 15 shadow warnings: flag every `Expr<T>`-kinded parameter whose
    // name collides with a column in any `TableExpr`-kinded parameter's
    // caller-supplied schema (§16 #1). The warning anchors at the
    // parameter's declaration span inside the signature. Body still
    // typechecks because `function_params` resolve before FROM-scope
    // columns in `lookup_identifier`.
    diagnostics.extend(compute_shadow_warnings(&sig, &body_ctx));

    // Phase 15: shadow warnings are not cascade-causing errors. Partition
    // them out so they don't suppress the body re-walk below.
    let shadow_warnings: Vec<Diagnostic> = diagnostics
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::ParameterShadowsColumn))
        .cloned()
        .collect();
    diagnostics.retain(|d| d.code != Some(DiagnosticCode::ParameterShadowsColumn));

    // If there were any non-warning call-site diagnostics
    // (unknown/missing/type-mismatch), stop here — re-walking the body
    // would cascade errors that are already subsumed by the call-site
    // issue. Shadow warnings are re-appended at the tail so they survive.
    if !diagnostics.is_empty() {
        diagnostics.extend(shadow_warnings);
        return diagnostics;
    }

    // Phase 25: Tier 2/3 *pure-scalar* bodies are checked at definition time —
    // skip expansion at call sites. Argument type-checking (step 3 above)
    // already ran. Re-walking the body here would only re-report errors that
    // were already surfaced at definition time (Phases 23/24).
    //
    // Exception: functions with `TableExpr` or `SelectItems` parameters still
    // need call-site expansion because their bodies reference caller-supplied
    // column schemas that are only known at the call site.
    let has_schema_param = sig.params.iter().any(|p| {
        matches!(
            &p.type_ref,
            Some(Ok(SmeltType::TableExpr(_))) | Some(Ok(SmeltType::SelectItems { .. }))
        )
    });
    if sig.tier != Tier::One && !has_schema_param {
        diagnostics.extend(shadow_warnings);
        return diagnostics;
    }

    // 4. Re-walk the body with the call-site-bound context. Errors surfaced
    //    here are re-anchored to the call site so the user sees the issue
    //    where they wrote the call, not inside the function they imported.
    //    The original span is preserved on the diagnostic — only the message
    //    range is rewritten — and a FrameInfo is attached.
    //
    //    Phase 12: the re-walk dispatches nested `smelt.fn.*` calls
    //    recursively through `check_smelt_fn_call`, so frames stack up
    //    across arbitrary expansion depth. Each level appends its frame
    //    after the inner-merged stack, yielding the canonical
    //    innermost-first → outermost-last ordering.
    let Some((body_text, body_shape)) = body_lookup(&sig) else {
        diagnostics.extend(shadow_warnings);
        return diagnostics;
    };

    let nested_handler = |nested_call: &SmeltFnCall,
                          nested_ctx: &TypeContext,
                          nested_text: &str|
     -> Vec<Diagnostic> {
        check_smelt_fn_call(
            nested_call,
            nested_ctx,
            nested_text,
            sig_lookup,
            builtin_lookup,
            lub,
            body_lookup,
            decl_lookup,
            tableexpr_schema_lookup,
            default_type_lookup,
        )
    };

    let inner = match &body_shape {
        BodyShape::Expression(body_expr) => {
            // Phase 26: Use the call-site-bound `body_ctx` (which has concrete arg
            // types from the caller) rather than re-seeding from the callee's
            // signature. For Tier 1 functions all params are unannotated → seeding
            // from the sig gives Unknown for every param, which suppresses type errors.
            // Using `body_ctx` directly gives each param its actual caller-supplied type.
            walk_body_with_ctx(body_expr, &body_ctx, &body_text, &nested_handler)
        }
        BodyShape::Select(select_stmt) => {
            // Phase 22: propagate the caller's workspace function signatures
            // into `body_ctx` so that `infer_cte_columns` can resolve nested
            // `smelt.fn.*` calls in CTE bodies (e.g. the `sessionize(...)` call
            // inside `session_rollup`'s `WITH sessionized AS (SELECT * FROM
            // smelt.fn.sessionize(...))`).  Without this, wildcard expansion
            // from a `smelt.fn.*` FROM source inside a CTE produces an empty
            // column list and downstream column references in the outer SELECT
            // emit false `UnknownIdentifier` errors.
            for (name, sig) in ctx.function_signatures_iter() {
                body_ctx.add_function_signature(name, sig.clone());
            }

            // Phase 22: seed CTE schemas from the function body's WITH
            // clause so that context-annotated `SelectItems<Kind, cte>`
            // parameters can resolve their column sets via `is_cte` /
            // `cte_columns`. CTE cycle diagnostics from this extraction
            // are discarded here — `cte_cycle_diagnostics_for_file`
            // handles them at definition time.
            let (body_ctx_with_ctes, _cycle_diags) =
                extract_function_body_cte_schemas(select_stmt, &body_ctx, &body_text);
            let body_ctx = body_ctx_with_ctes;

            // Phase 15: a SELECT-shaped body (e.g. `add_margin`'s
            // `(SELECT source.*, revenue - cost AS margin FROM source)`)
            // is checked with the TableExpr params' caller schemas
            // already seeded into `body_ctx`. We emit
            // `UnknownIdentifier` for any bare column / alias that
            // doesn't resolve; Phase 6+ nested-call traversal fires via
            // `walk_select_columns_with_visitor` so `smelt.fn.*` calls
            // inside the body still get checked recursively.
            let mut body_diags = check_function_select_body(
                &sig,
                select_stmt,
                &body_text,
                &body_ctx,
                &nested_handler,
            );
            // Phase 21: validate caller-provided Expr<T>/SelectItems<Kind>
            // fragment arguments against the inferred splice contexts.
            // Phase 22: `body_ctx` now includes CTE schemas so that
            // `SelectItems<Kind, cte_name>` parameters can validate
            // caller fragments against the CTE's column set.
            body_diags.extend(check_fragment_context_bindings(
                &sig,
                select_stmt,
                &body_ctx,
                &bindings,
                text,
            ));
            body_diags
        }
    };

    // Resolve this frame's decl-site info once — reused for every
    // cascading diagnostic. LSP clients use these fields to render a
    // `DiagnosticRelatedInformation` link per frame (§16 #16).
    let decl_path = decl_lookup(&sig);
    let decl_range = Some(sig.name_range);
    let call_site_range = Some(path_range);

    // Build the frames list. For each *inner* diagnostic we push:
    //   - any frames it already carried (from nested calls), unchanged
    //   - plus one new frame for `this` call site, appended to the end so
    //     the last element is the outermost (current) call — matching the
    //     renderer contract in `smelt-lsp::to_lsp_diagnostic`.
    for mut d in inner {
        // Re-anchor the range to the call-site call-path span so the
        // editor squiggle lands where the user wrote the call. The
        // original inner anchor is preserved on the corresponding frame
        // via `call_site_range` for LSP related-info (Phase 12).
        d.range = path_range;

        // Merge any pre-existing ExpansionFrames with this call's frame.
        let mut frames: Vec<FrameInfo> = match d.data.take() {
            Some(DiagnosticData::ExpansionFrames(existing)) => existing,
            _ => Vec::new(),
        };
        // Phase 6 packs all this call's (param, bound_type) pairs into a
        // single frame keyed on the function name. The renderer only shows
        // one binding per frame — pick the first parameter (innermost-bound)
        // for determinism. Future phases can extend FrameInfo with a full
        // binding list.
        if let Some((param_name, bound_type)) = frame_bindings.first().cloned() {
            frames.push(FrameInfo {
                function: sig.name.clone(),
                param: param_name,
                bound_type,
                decl_path: decl_path.clone(),
                decl_range,
                call_site_range,
            });
        } else {
            frames.push(FrameInfo {
                function: sig.name.clone(),
                param: String::new(),
                bound_type: String::new(),
                decl_path: decl_path.clone(),
                decl_range,
                call_site_range,
            });
        }
        d.data = Some(DiagnosticData::ExpansionFrames(frames));
        diagnostics.push(d);
    }

    // Phase 15: re-append the shadow warnings we set aside above —
    // they're not cascade errors so they must survive even when inner
    // body-check diagnostics are empty.
    diagnostics.extend(shadow_warnings);

    diagnostics
}

/// Infer the output schema of a `TableExpr`-returning `smelt.define`
/// body (Phase 17).
///
/// Walks the top-level SELECT's projection list and builds the output
/// column list by:
///   1. Expanding `source.*` (qualified wildcard) against the
///      `TableExpr` parameter schema seeded on `ctx` (via
///      [`TypeContext::add_tableexpr_param`]). Bare `*` is treated the
///      same way when there is exactly one `TableExpr` parameter in
///      scope — the common case.
///   2. Adding each explicit `expr AS name` projection as a single
///      column, inferring the column's type via
///      [`infer_expression_type`].
///   3. Adding bare-column projections (`col_ref`) using the
///      column-ref's name and inferred type.
///
/// Returns `None` when the body has no top-level SELECT list
/// (shouldn't happen for well-formed `TableExpr` bodies but the CST
/// may hand back a malformed AST under error recovery).
///
/// Pure — no Salsa. Callers that want the rendered model schema
/// attach it as a FROM-scope entry on the enclosing caller model's
/// context so bare-column references like `SELECT margin FROM
/// smelt.fn.add_margin(…)` type-check.
pub fn infer_tableexpr_return_schema(body: &SelectStmt, ctx: &TypeContext) -> Option<ModelSchema> {
    let select_list = body.select_list()?;
    let mut columns: Vec<Column> = Vec::new();

    // Collect every TableExpr parameter's schema once. Used for both
    // qualified (`source.*`) and bare (`*` with exactly one table-expr
    // param) wildcard expansion.
    let tableexpr_params: Vec<(String, Vec<(String, TypedColumn)>)> = ctx
        .tableexpr_param_schemas_iter()
        .map(|(name, cols)| (name.to_string(), cols.to_vec()))
        .collect();

    for item in select_list.items() {
        // Qualified wildcard: expand against the named TableExpr param
        // (or any FROM-scope alias matching the qualifier).
        if let Some(qualifier) = item.qualified_wildcard_target() {
            if let Some((_, cols)) = tableexpr_params.iter().find(|(n, _)| *n == qualifier) {
                for (col_name, typed_col) in cols {
                    columns.push(Column {
                        name: col_name.clone(),
                        alias: None,
                        source: ColumnSource::Computed,
                        expression: format!("{}.{}", qualifier, col_name),
                        range: item.range(),
                        data_type: Some(typed_col.clone()),
                    });
                }
            }
            continue;
        }
        // Bare `*`: when there is exactly one TableExpr param, expand
        // against it. For multiple params the ambiguity resolution is
        // out of scope for Phase 17 — leave those as unknown.
        if item.is_wildcard() {
            if tableexpr_params.len() == 1 {
                let (_, cols) = &tableexpr_params[0];
                for (col_name, typed_col) in cols {
                    columns.push(Column {
                        name: col_name.clone(),
                        alias: None,
                        source: ColumnSource::Computed,
                        expression: "*".to_string(),
                        range: item.range(),
                        data_type: Some(typed_col.clone()),
                    });
                }
            }
            continue;
        }

        // Explicit projection (with or without alias).
        let Some(expr) = item.expression() else {
            continue;
        };
        let name = item.column_name().unwrap_or_else(|| "?col?".to_string());
        let typed = infer_expression_type(&expr, ctx);
        let alias = item.alias();
        columns.push(Column {
            name,
            alias,
            source: ColumnSource::Computed,
            expression: expr.text(),
            range: item.range(),
            data_type: typed,
        });
    }

    Some(ModelSchema {
        columns,
        row_extensions: Vec::new(),
        input_constraints: Vec::new(),
    })
}

/// Walk a SELECT-shaped function body (Phase 15) against a pre-seeded
/// body context and produce body-level diagnostics.
///
/// `body_ctx` must already have any `TableExpr` parameter schemas seeded
/// via [`TypeContext::add_tableexpr_param`] and any `Expr<T>`
/// parameters seeded via [`TypeContext::add_function_param`]. The
/// pre-existing SELECT-walker infrastructure handles:
///   - bare-column resolution through the param-first/FROM-scope chain;
///   - unresolved identifiers surface as `UnknownIdentifier` anchored at
///     the usage site (Phase 15 §16 #7);
///   - nested `smelt.fn.*` calls inside the SELECT-list expressions
///     dispatch through `nested_handler` so frames stack across
///     expansion depth (Phase 12 contract).
fn check_function_select_body(
    _sig: &FunctionSig,
    select_stmt: &SelectStmt,
    text: &str,
    body_ctx: &TypeContext,
    nested_handler: &NestedCallHandler<'_>,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    // 1. Bare-column / qualified-access resolution. Any identifier the
    //    SELECT references that does not resolve in `body_ctx` emits
    //    `UnknownIdentifier`. Select-list aliases are handled by
    //    `check_undeclared_columns` already.
    for info in check_undeclared_columns(select_stmt, body_ctx) {
        diagnostics.push(Diagnostic {
            severity: DiagnosticSeverity::Error,
            message: format!(
                "Unknown identifier `{}` — not a parameter or in any enclosing scope",
                info.column_name
            ),
            range: to_range(info.range, text),
            code: Some(DiagnosticCode::UnknownIdentifier),
            data: None,
        });
    }

    // 2. Dispatch nested `smelt.fn.*` calls so frames stack up across
    //    expansion depth. We walk every expression in the SELECT and
    //    hand any SMELT_FN_CALL node to `nested_handler`.
    walk_select_columns_with_visitor(
        select_stmt,
        body_ctx,
        None,
        &mut |_qualifier, _name, _expr_type, _range| {
            // `walk_select_columns_with_visitor` hits leaf column refs,
            // not nested function calls. We walk SMELT_FN_CALL nodes
            // separately below.
        },
    );

    // Walk every `SMELT_FN_CALL` in the SELECT statement and let the
    // nested handler produce diagnostics with merged frame stacks.
    use smelt_parser::syntax_kind::SyntaxKind;
    for node in select_stmt.syntax().descendants() {
        if node.kind() == SyntaxKind::SMELT_FN_CALL {
            if let Some(call) = SmeltFnCall::cast(node) {
                diagnostics.extend(nested_handler(&call, body_ctx, text));
            }
        }
    }

    diagnostics
}

/// Compute Phase-15 shadow warnings: for each `Expr<T>`-kinded parameter
/// whose name matches a column in any `TableExpr`-kinded parameter's
/// caller-supplied schema, emit a warning anchored at the parameter's
/// declaration span.
///
/// §16 #1: parameters shadow FROM-scope columns, so the body still
/// typechecks, but the user probably meant the column — hence a warning,
/// not an error.
fn compute_shadow_warnings(sig: &FunctionSig, body_ctx: &TypeContext) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    // Collect all column names supplied by TableExpr parameters.
    let mut column_set: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (_param, cols) in body_ctx.tableexpr_param_schemas_iter() {
        for (col_name, _) in cols {
            column_set.insert(col_name.clone());
        }
    }
    if column_set.is_empty() {
        return out;
    }
    for param in &sig.params {
        if param.name.is_empty() {
            continue;
        }
        // Only `Expr<T>`-kinded parameters shadow FROM-scope columns
        // (TableExpr parameters BECOME the FROM-scope; they don't
        // shadow).
        if is_tableexpr_param(param) {
            continue;
        }
        if !column_set.contains(&param.name) {
            continue;
        }
        let Some(range) = param.name_range else {
            continue;
        };
        out.push(Diagnostic {
            severity: DiagnosticSeverity::Warning,
            message: format!(
                "Parameter `{}` shadows a column of the same name in the caller-supplied table schema. \
                 Inside the body, the bare identifier resolves to the parameter; use a qualified reference (e.g. `<table>.{name}`) to access the column.",
                param.name,
                name = param.name,
            ),
            range,
            code: Some(DiagnosticCode::ParameterShadowsColumn),
            data: None,
        });
    }
    out
}

/// Dispatch a `smelt.fn.<name>(...)` call against a built-in registry
/// [`Signature`] via [`unify_call`] (Phase 10 unified-resolver path).
///
/// This is reached when `sig_lookup` misses but `builtin_lookup` hits —
/// i.e. the user is calling a built-in like `COALESCE`/`GREATEST` through
/// the `smelt.fn.*` syntax.
///
/// Mapped diagnostics (all pinned to the smallest useful span):
///   - [`UnificationError::ConstraintViolation`] →
///     [`DiagnosticCode::ArgTypeMismatch`] at the offending positional arg.
///   - [`UnificationError::MissingArgs`] →
///     [`DiagnosticCode::MissingArgument`] at the call-path span.
///   - [`UnificationError::TooManyArgs`] → ignored in Phase 10 (no
///     too-many diagnostic code today); silently accepted.
///   - [`UnificationError::InconsistentBinding`] →
///     [`DiagnosticCode::ArgTypeMismatch`] at the first conflicting arg.
///   - [`UnificationError::EmptyVariadicTypeVar`] → ignored (cannot bind
///     without surrounding context; Phase 12+).
///
/// Named arguments are not modelled on built-ins (the registry is
/// positional-only), so any `name => value` pairs are ignored for
/// unification and named-value spans fall back to positional order.
fn check_builtin_call(
    call: &SmeltFnCall,
    name: &str,
    path_range: Range,
    sig: &Signature,
    ctx: &TypeContext,
    text: &str,
    lub: &dyn Fn(&DataType, &DataType) -> DataType,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    // Collect positional arg expressions + their inferred DataTypes.
    // Named args on built-ins have no declared parameter names in the
    // registry, so they contribute positionally if present but are rare —
    // callers use `smelt.fn.COALESCE(a, b)` style. We treat named-arg
    // value exprs as trailing positional args, keeping order stable.
    let arg_list = call.arg_list();
    let positional_exprs: Vec<Expr> = arg_list
        .as_ref()
        .map(|al| al.positional_args())
        .unwrap_or_default();
    let named_values: Vec<Expr> = arg_list
        .as_ref()
        .map(|al| al.named_params().filter_map(|np| np.value_expr()).collect())
        .unwrap_or_default();

    let mut arg_exprs: Vec<Expr> = Vec::with_capacity(positional_exprs.len() + named_values.len());
    arg_exprs.extend(positional_exprs);
    arg_exprs.extend(named_values);

    let mut arg_types: Vec<DataType> = Vec::with_capacity(arg_exprs.len());
    for arg in &arg_exprs {
        let dt = infer_expression_type(arg, ctx)
            .map(|t| t.data_type)
            .unwrap_or(DataType::Unknown);
        arg_types.push(dt);
    }

    match unify_call(sig, &arg_types, lub) {
        Ok(_) => {} // no diagnostics to emit — type is already inferred elsewhere
        Err(UnificationError::MissingArgs { expected, got }) => {
            diagnostics.push(Diagnostic {
                severity: DiagnosticSeverity::Error,
                message: format!(
                    "`smelt.fn.{}` expects at least {} argument(s), got {}",
                    name, expected, got
                ),
                range: path_range,
                code: Some(DiagnosticCode::MissingArgument),
                data: None,
            });
        }
        Err(UnificationError::ConstraintViolation {
            position,
            param_constraint,
            actual,
        }) => {
            // `position` is 1-based — index into our flat positional list.
            let arg_range = arg_exprs
                .get(position.saturating_sub(1))
                .map(|e| to_range(e.text_range(), text))
                .unwrap_or(path_range);
            diagnostics.push(Diagnostic {
                severity: DiagnosticSeverity::Error,
                message: format!(
                    "Argument at position {} has type `{}`, which does not satisfy constraint `{}` of `smelt.fn.{}`",
                    position,
                    actual,
                    format_constraint(&param_constraint),
                    name
                ),
                range: arg_range,
                code: Some(DiagnosticCode::ArgTypeMismatch),
                data: None,
            });
        }
        Err(UnificationError::InconsistentBinding {
            var_name,
            positions,
            types,
        }) => {
            // Anchor at the second inconsistent position if possible — the
            // first one set the binding, the second one violated it.
            let anchor_pos = positions.get(1).copied().unwrap_or(1);
            let arg_range = arg_exprs
                .get(anchor_pos.saturating_sub(1))
                .map(|e| to_range(e.text_range(), text))
                .unwrap_or(path_range);
            let type_list = types
                .iter()
                .map(|t| t.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            diagnostics.push(Diagnostic {
                severity: DiagnosticSeverity::Error,
                message: format!(
                    "Type variable `{}` in `smelt.fn.{}` inferred inconsistently across positions {:?}: {}",
                    var_name, name, positions, type_list
                ),
                range: arg_range,
                code: Some(DiagnosticCode::ArgTypeMismatch),
                data: None,
            });
        }
        Err(UnificationError::EmptyVariadicTypeVar { .. }) => {
            // A variadic built-in (`GREATEST`, `LEAST`, `COALESCE`) with no
            // arguments leaves its type variable unbound. Surface this as a
            // `MissingArgument` anchored at the call-path — the most
            // actionable interpretation since the fix is "supply at least
            // one arg".
            diagnostics.push(Diagnostic {
                severity: DiagnosticSeverity::Error,
                message: format!(
                    "`smelt.fn.{}` is variadic and requires at least one argument",
                    name
                ),
                range: path_range,
                code: Some(DiagnosticCode::MissingArgument),
                data: None,
            });
        }
        Err(UnificationError::TooManyArgs { .. }) => {
            // Phase 10 does not emit a too-many-args diagnostic; callers
            // see no diagnostic and fall through to legacy inference.
        }
    }

    diagnostics
}

/// Render a [`TypeConstraint`] in a user-facing form for diagnostic
/// messages. Mirrors the `Expr<…>` vocabulary users see on `smelt.define`
/// parameter annotations so error text stays consistent.
fn format_constraint(c: &TypeConstraint) -> String {
    match c {
        TypeConstraint::Concrete(dt) => format!("Expr<{}>", dt),
        TypeConstraint::Numeric => "Expr<Numeric>".to_string(),
        TypeConstraint::Ordered => "Expr<Ordered>".to_string(),
        TypeConstraint::Any => "Expr<Any>".to_string(),
    }
}

/// Minimal assignment-compatibility check used by `ArgTypeMismatch`.
///
/// For Phase 6 we want:
///   - Exact match: always OK.
///   - Unknown / Null: skip (caller handles).
///   - Integer-family promotion is allowed in one direction only — passing an
///     Integer where a BigInt is expected is accepted (a SUM of Ints → BigInt
///     path), but passing a BigInt where Integer is expected is flagged.
///   - Text / Varchar: interchangeable (normalize() maps them).
///   - Anything else: strict equality.
fn types_assignment_compatible(expected: &DataType, actual: &DataType) -> bool {
    if expected == actual {
        return true;
    }
    // Normalize both via DataType::normalize() to collapse Text ↔ Varchar.
    if expected.normalize() == actual.normalize() {
        return true;
    }
    // Allow widening an Integer-family actual into a wider Integer-family
    // expected. Do NOT accept Double → BigInt (lossy) or Numeric → Text.
    use DataType::*;
    matches!(
        (expected, actual),
        (BigInt, SmallInt) | (BigInt, Integer) | (Integer, SmallInt)
    )
}

// ============================================================================
// Phase 20 — CTE schema extraction + splice-point context inference
// ============================================================================

/// Colour used by the DFS cycle-detection pass (white=unvisited,
/// grey=in-progress, black=done).
#[derive(Clone, Copy, PartialEq, Eq)]
enum DfsColour {
    White,
    Grey,
    Black,
}

/// DFS state for extracting CTE schemas with cycle detection (Pass 1).
struct CteDfs<'a> {
    /// CTE name → Cte node for all CTEs in the WITH clause.
    ctes: std::collections::HashMap<String, Cte>,
    /// Set of all CTE names (for dependency look-up).
    all_names: std::collections::HashSet<String>,
    /// Colour of each CTE node in the DFS.
    colour: std::collections::HashMap<String, DfsColour>,
    /// Topological processing result: (cte_name, columns).
    topo: Vec<(String, Vec<(String, TypedColumn)>)>,
    /// Cycle diagnostics emitted during the DFS.
    diagnostics: Vec<Diagnostic>,
    /// The seed context — parameters and outer CTEs already in scope.
    seed_ctx: &'a TypeContext,
    /// Source text for anchoring diagnostics.
    text: &'a str,
}

impl<'a> CteDfs<'a> {
    fn visit(&mut self, name: &str) {
        let colour = self.colour.get(name).copied().unwrap_or(DfsColour::White);

        if colour == DfsColour::Black {
            return;
        }
        if colour == DfsColour::Grey {
            // Back-edge: cycle detected.
            let range = self
                .ctes
                .get(name)
                .and_then(|c| c.name_range())
                .map(|tr| to_range(tr, self.text))
                .unwrap_or(Range {
                    start: smelt_parser::Position { line: 0, column: 0 },
                    end: smelt_parser::Position { line: 0, column: 0 },
                });
            self.diagnostics.push(Diagnostic {
                severity: DiagnosticSeverity::Error,
                message: format!(
                    "CTE `{name}` forms a cyclic reference (directly or transitively)"
                ),
                range,
                code: Some(DiagnosticCode::CteCycle),
                data: None,
            });
            return;
        }

        self.colour.insert(name.to_string(), DfsColour::Grey);

        // Find this CTE's dependencies on other CTEs in the same WITH clause.
        let deps: Vec<String> = self
            .ctes
            .get(name)
            .map(|c| find_cte_table_deps(c, &self.all_names))
            .unwrap_or_default();

        for dep in deps {
            self.visit(&dep.clone());
        }

        // Build context from already-processed CTEs.
        let mut ctx = self.seed_ctx.clone();
        for (cte_nm, cols) in &self.topo {
            for (col, typed) in cols {
                ctx.add_cte_column(cte_nm, col, typed.clone());
            }
            ctx.add_alias(cte_nm, cte_nm);
        }

        // Infer this CTE's schema.
        if let Some(cte) = self.ctes.get(name) {
            let cols = infer_cte_columns(cte, &ctx);
            self.topo.push((name.to_string(), cols));
        }

        self.colour.insert(name.to_string(), DfsColour::Black);
    }
}

/// Walk the direct FROM-clause table references of a CTE body and return the
/// names of any that match a known CTE in the same WITH clause.
fn find_cte_table_deps(cte: &Cte, all_names: &std::collections::HashSet<String>) -> Vec<String> {
    let Some(query) = cte.query() else {
        return vec![];
    };
    let Some(select) = query.select_stmt() else {
        return vec![];
    };
    let mut deps: Vec<String> = vec![];
    for node in select.syntax().descendants() {
        if node.kind() == smelt_parser::SyntaxKind::TABLE_REF {
            if let Some(tr) = smelt_parser::ast::TableRef::cast(node) {
                if !tr.is_function_call() && tr.smelt_fn_call().is_none() {
                    if let Some(id) = tr.identifier() {
                        if all_names.contains(&id) && !deps.contains(&id) {
                            deps.push(id);
                        }
                    }
                }
            }
        }
    }
    deps
}

/// Phase 20 Pass 1: extract CTE schemas from a SELECT body's WITH clause.
///
/// Uses depth-first search with colour-based cycle detection. Returns:
/// - A clone of `seed_ctx` augmented with all CTE schemas (CTE columns +
///   aliases added via [`TypeContext::add_cte_column`]).
/// - Any [`DiagnosticCode::CteCycle`] diagnostics encountered.
///
/// When no WITH clause is present the seed context is returned unchanged.
/// Pure — does not touch Salsa.
pub fn extract_function_body_cte_schemas(
    select: &SelectStmt,
    seed_ctx: &TypeContext,
    text: &str,
) -> (TypeContext, Vec<Diagnostic>) {
    let Some(with_clause) = select.with_clause() else {
        return (seed_ctx.clone(), vec![]);
    };

    let ctes: std::collections::HashMap<String, Cte> = with_clause
        .ctes()
        .filter_map(|c| c.name().map(|n| (n, c)))
        .collect();

    if ctes.is_empty() {
        return (seed_ctx.clone(), vec![]);
    }

    let all_names: std::collections::HashSet<String> = ctes.keys().cloned().collect();
    let names_iter: Vec<String> = ctes.keys().cloned().collect();

    let mut dfs = CteDfs {
        ctes,
        all_names,
        colour: std::collections::HashMap::new(),
        topo: vec![],
        diagnostics: vec![],
        seed_ctx,
        text,
    };

    for name in names_iter {
        dfs.visit(&name);
    }

    // Build the final augmented context from the topological result.
    let mut ctx = seed_ctx.clone();
    for (cte_name, cols) in &dfs.topo {
        for (col_name, typed_col) in cols {
            ctx.add_cte_column(cte_name, col_name, typed_col.clone());
        }
        ctx.add_alias(cte_name, cte_name);

        // Phase 22: if this CTE SELECTs from a `smelt.fn.*` source with a
        // wildcard projection, its output schema cannot be determined at
        // pure-function-check time (wildcard expansion from a
        // user-defined-function source requires the function's body AST,
        // which is not available here). Mark it as opaque so that column
        // references from this CTE in the outer SELECT don't emit
        // `UnknownIdentifier` false positives.
        {
            let is_wildcard_from_smelt_fn = dfs
                .ctes
                .get(cte_name)
                .and_then(|c| c.query())
                .and_then(|q| q.select_stmt())
                .map(|s| {
                    // Has a FROM clause with a smelt.fn.* source
                    let has_smelt_fn_from = s
                        .from_clause()
                        .map(|fc| fc.table_refs().any(|tr| tr.smelt_fn_call().is_some()))
                        .unwrap_or(false);
                    // AND the SELECT list contains a wildcard
                    let has_wildcard = s
                        .select_list()
                        .map(|sl| sl.items().any(|item| item.is_wildcard()))
                        .unwrap_or(false);
                    has_smelt_fn_from && has_wildcard
                })
                .unwrap_or(false);
            if is_wildcard_from_smelt_fn {
                ctx.mark_cte_opaque(cte_name);
            }
        }
    }

    (ctx, dfs.diagnostics)
}

/// Return `true` when any unqualified IDENT token in `expr` exactly matches
/// `param_name`.
///
/// Used by [`infer_splice_contexts`] to detect where an `Expr<T>` parameter
/// is referenced inside a WHERE or HAVING clause.
pub fn expr_refs_param(expr: &Expr, param_name: &str) -> bool {
    for token in expr
        .syntax()
        .descendants_with_tokens()
        .filter_map(|e| e.into_token())
    {
        if token.kind() == smelt_parser::SyntaxKind::IDENT && token.text() == param_name {
            return true;
        }
    }
    false
}

/// Collect the FROM-clause columns that are in scope at the WHERE clause
/// for this SELECT.
///
/// Walks the FROM clause and, for each non-function table ref, looks up the
/// ref name in `body_ctx` as either a `TableExpr` parameter (via
/// [`TypeContext::tableexpr_param_columns`]) or a CTE (via
/// [`TypeContext::cte_columns`]).  All matching columns are returned.
fn from_scope_columns(select: &SelectStmt, body_ctx: &TypeContext) -> Vec<(String, TypedColumn)> {
    let Some(from_clause) = select.from_clause() else {
        return vec![];
    };
    let mut cols: Vec<(String, TypedColumn)> = vec![];
    for table_ref in from_clause.table_refs() {
        if table_ref.is_function_call() || table_ref.smelt_fn_call().is_some() {
            continue;
        }
        let Some(id) = table_ref.identifier() else {
            continue;
        };
        // TableExpr param?
        if let Some(param_cols) = body_ctx.tableexpr_param_columns(&id) {
            for (col_name, typed) in param_cols {
                cols.push((col_name.clone(), typed.clone()));
            }
        }
        // CTE?
        else if body_ctx.is_cte(&id) {
            for (col_name, typed) in body_ctx.cte_columns(&id) {
                cols.push((col_name.to_string(), typed.clone()));
            }
        }
    }
    cols
}

/// Intersect two column schemas by name.  The result contains only columns
/// whose names appear in BOTH inputs.
fn intersect_columns(
    a: &[(String, TypedColumn)],
    b: &[(String, TypedColumn)],
) -> Vec<(String, TypedColumn)> {
    let b_names: std::collections::HashSet<&str> = b.iter().map(|(n, _)| n.as_str()).collect();
    a.iter()
        .filter(|(n, _)| b_names.contains(n.as_str()))
        .cloned()
        .collect()
}

/// Phase 20 Pass 2 (splice-point tracker): for each `Expr<T>` parameter in
/// `sig` that is NOT a `TableExpr`, find where it is referenced in `select`'s
/// WHERE and HAVING clauses and return the intersection of the FROM-scope
/// column sets at each splice point.
///
/// - **WHERE scope**: columns from the FROM clause's `TableExpr` params / CTEs
///   (as seeded in `body_ctx`).
/// - **HAVING scope**: projected columns from the SELECT list (inferred via
///   [`infer_tableexpr_return_schema`]).
/// - **Intersection**: field-by-field by column name over all splice points
///   for the same parameter.
///
/// Returns a map `param_name → inferred_columns`.  Parameters that do not
/// appear in any splice point are absent from the map.
///
/// Pure — does not touch Salsa.
pub fn infer_splice_contexts(
    sig: &FunctionSig,
    select: &SelectStmt,
    body_ctx: &TypeContext,
) -> std::collections::HashMap<String, Vec<(String, TypedColumn)>> {
    let mut result: std::collections::HashMap<String, Vec<(String, TypedColumn)>> =
        std::collections::HashMap::new();

    // Only consider Expr<T> params (not TableExpr).
    let expr_params: Vec<&str> = sig
        .params
        .iter()
        .filter(|p| !is_tableexpr_param(p))
        .map(|p| p.name.as_str())
        .collect();

    if expr_params.is_empty() {
        return result;
    }

    // Pre-compute the WHERE scope = FROM clause columns.
    let where_scope = from_scope_columns(select, body_ctx);

    // Pre-compute the HAVING scope = SELECT-list projected columns.
    let having_scope: Vec<(String, TypedColumn)> = infer_tableexpr_return_schema(select, body_ctx)
        .map(|schema| {
            schema
                .columns
                .iter()
                .map(|c| {
                    (
                        c.name.clone(),
                        c.data_type
                            .clone()
                            .unwrap_or_else(|| TypedColumn::nullable(DataType::Unknown)),
                    )
                })
                .collect()
        })
        .unwrap_or_default();

    for param_name in expr_params {
        let mut scopes: Vec<Vec<(String, TypedColumn)>> = vec![];

        // Check WHERE clause.
        if let Some(wc) = select.where_clause() {
            if let Some(expr) = wc.expression() {
                if expr_refs_param(&expr, param_name) && !where_scope.is_empty() {
                    scopes.push(where_scope.clone());
                }
            }
        }

        // Check HAVING clause.
        if let Some(hc) = select.having_clause() {
            if let Some(expr) = hc.expression() {
                if expr_refs_param(&expr, param_name) && !having_scope.is_empty() {
                    scopes.push(having_scope.clone());
                }
            }
        }

        if scopes.is_empty() {
            continue;
        }

        // Intersect all recorded scopes.
        let inferred = scopes[1..]
            .iter()
            .fold(scopes[0].clone(), |acc, s| intersect_columns(&acc, s));

        if !inferred.is_empty() {
            result.insert(param_name.to_string(), inferred);
        }
    }

    result
}

/// Definition-time ContextMismatch check for a single `smelt.define` with a
/// SELECT body.
///
/// For each `Expr<T, ctx_name>` parameter:
///   1. Check whether the parameter name appears in a WHERE clause.
///   2. If so, identify the `TableExpr` parameter directly named in the FROM
///      clause.
///   3. If the FROM parameter name differs from `ctx_name` → emit
///      [`DiagnosticCode::ContextMismatch`].
///
/// This is a name-based check (no column schemas needed) so it works at
/// definition time before any caller context is available.
///
/// Pure — does not touch Salsa.
pub fn context_mismatch_diagnostics_for_fn(
    sig: &FunctionSig,
    select: &SelectStmt,
) -> Vec<Diagnostic> {
    // Find the primary TableExpr param named in the FROM clause.
    let inferred_from_param: Option<String> = {
        let tableexpr_names: std::collections::HashSet<&str> = sig
            .params
            .iter()
            .filter(|p| is_tableexpr_param(p))
            .map(|p| p.name.as_str())
            .collect();

        select.from_clause().and_then(|fc| {
            fc.table_refs()
                .filter(|tr| !tr.is_function_call() && tr.smelt_fn_call().is_none())
                .find_map(|tr| {
                    tr.identifier()
                        .filter(|id| tableexpr_names.contains(id.as_str()))
                })
        })
    };

    let Some(ref from_param) = inferred_from_param else {
        // Can't infer — skip mismatch check.
        return vec![];
    };

    let mut out: Vec<Diagnostic> = vec![];

    for param in &sig.params {
        let Some(ctx_ref) = &param.context else {
            continue;
        };
        let ctx_name = ctx_ref.name();

        // Only fire when the param actually appears in a WHERE clause.
        let in_where = select
            .where_clause()
            .and_then(|wc| wc.expression())
            .map(|e| expr_refs_param(&e, &param.name))
            .unwrap_or(false);
        if !in_where {
            continue;
        }

        if ctx_name != from_param.as_str() {
            let Some(range) = param.type_ref_range else {
                continue;
            };
            out.push(Diagnostic {
                severity: DiagnosticSeverity::Error,
                message: format!(
                    "Context annotation `{ctx_name}` for parameter `{}` does not match \
                     the inferred splice context `{from_param}` in `{}`",
                    param.name, sig.name
                ),
                range,
                code: Some(DiagnosticCode::ContextMismatch),
                data: None,
            });
        }
    }

    out
}

/// Phase 21: At a `smelt.fn.*` call site, validate caller-provided `Expr<T>`
/// and `SelectItems<Kind>` argument fragments against their inferred splice
/// contexts.
///
/// For each non-`TableExpr` parameter with a caller-supplied argument:
///
/// 1. **Kind check** (`SelectItems<Kind, …>` params): the argument's
///    [`ExprKind`] must not be strictly less than the declared kind ceiling.
///    Scalar arguments for `SelectItems<Agg>` emit
///    [`DiagnosticCode::FragmentKindMismatch`].
///
/// 2. **Annotation-vs-inference check** (`Expr<T, ctx_name>` params): the
///    column set of `ctx_name` in `body_ctx` must be a *subset* of the
///    inferred splice context (from [`infer_splice_contexts`]). Extra columns
///    in the annotation that are not in the inferred context emit
///    [`DiagnosticCode::AnnotationTooWide`] at the argument span.
///
/// 3. **Fragment column check**: column references inside the argument
///    expression must resolve against the inferred splice context columns.
///    Unknown column references emit [`DiagnosticCode::FragmentColumnMissing`].
///
/// `body_ctx` must already be seeded with the call-site [`TableExpr`] schemas
/// (as `check_smelt_fn_call` does before calling this function).
/// `text` is the call-site source text used to convert [`TextRange`]s to
/// [`Range`]s.
///
/// Pure — does not touch Salsa.
pub fn check_fragment_context_bindings(
    sig: &FunctionSig,
    select: &SelectStmt,
    body_ctx: &TypeContext,
    bindings: &std::collections::HashMap<String, (Expr, TextRange)>,
    text: &str,
) -> Vec<Diagnostic> {
    let inferred = infer_splice_contexts(sig, select, body_ctx);
    let mut out = Vec::new();

    for param in &sig.params {
        if is_tableexpr_param(param) {
            continue;
        }

        // 1. Kind check for SelectItems<Kind> parameters.
        if let Some(Ok(SmeltType::SelectItems { kind: req_kind, .. })) = &param.type_ref {
            if let Some((arg_expr, arg_range)) = bindings.get(&param.name) {
                let found_kind = infer_expression_kind(arg_expr, body_ctx);
                let kind_ok = match req_kind {
                    ExprKind::Scalar => true,
                    ExprKind::Agg => matches!(found_kind, ExprKind::Agg | ExprKind::Window),
                    ExprKind::Window => matches!(found_kind, ExprKind::Window),
                };
                if !kind_ok {
                    let req_str = match req_kind {
                        ExprKind::Scalar => "Scalar",
                        ExprKind::Agg => "Agg",
                        ExprKind::Window => "Window",
                    };
                    let found_str = match found_kind {
                        ExprKind::Scalar => "Scalar",
                        ExprKind::Agg => "Agg",
                        ExprKind::Window => "Window",
                    };
                    out.push(Diagnostic {
                        severity: DiagnosticSeverity::Error,
                        message: format!(
                            "Argument for `{}` in `{}` must be {}-kind or higher, \
                             but found {}-kind expression",
                            param.name, sig.name, req_str, found_str
                        ),
                        range: to_range(*arg_range, text),
                        code: Some(DiagnosticCode::FragmentKindMismatch),
                        data: None,
                    });
                }
            }
        }

        // Determine the effective inferred column set for this parameter.
        // For SelectItems<Kind, ctx_name>, use the ctx_name schema directly.
        // Phase 22: ctx_name may refer to a CTE in the function body rather
        // than a TableExpr parameter — check both.
        // For Expr<T> params, use the splice-point inferred context.
        let effective_inferred: Option<Vec<(String, TypedColumn)>> =
            if let Some(Ok(SmeltType::SelectItems {
                context: Some(ContextRef(ctx_name)),
                ..
            })) = &param.type_ref
            {
                body_ctx
                    .tableexpr_param_columns(ctx_name)
                    .map(|cols| cols.to_vec())
                    .or_else(|| {
                        if body_ctx.is_cte(ctx_name) {
                            Some(
                                body_ctx
                                    .cte_columns(ctx_name)
                                    .into_iter()
                                    .map(|(n, t)| (n.to_string(), t.clone()))
                                    .collect(),
                            )
                        } else {
                            None
                        }
                    })
            } else {
                inferred.get(&param.name).cloned()
            };

        let Some(inferred_cols) = effective_inferred else {
            continue;
        };

        let inferred_set: std::collections::HashSet<String> =
            inferred_cols.iter().map(|(n, _)| n.clone()).collect();

        // 2. Annotation-vs-inference check for Expr<T, ctx_name> parameters.
        if let Some(ContextRef(ctx_name)) = &param.context {
            if let Some(ann_cols) = body_ctx.tableexpr_param_columns(ctx_name) {
                let wider: Vec<&str> = ann_cols
                    .iter()
                    .filter(|(n, _)| !inferred_set.contains(n.as_str()))
                    .map(|(n, _)| n.as_str())
                    .collect();
                if !wider.is_empty() {
                    if let Some((_, arg_range)) = bindings.get(&param.name) {
                        out.push(Diagnostic {
                            severity: DiagnosticSeverity::Error,
                            message: format!(
                                "Annotation `{}` for `{}` in `{}` is wider than the inferred \
                                 splice context: columns `{}` are declared but not available \
                                 at the splice point",
                                ctx_name,
                                param.name,
                                sig.name,
                                wider.join(", ")
                            ),
                            range: to_range(*arg_range, text),
                            code: Some(DiagnosticCode::AnnotationTooWide),
                            data: None,
                        });
                    }
                }
            }
        }

        // 3. Fragment column validation.
        if let Some((arg_expr, _)) = bindings.get(&param.name) {
            walk_expression_columns_with_visitor(
                arg_expr,
                body_ctx,
                None,
                &mut |_qualifier, col_name, _, col_range| {
                    let lower = col_name.to_lowercase();
                    if matches!(lower.as_str(), "true" | "false" | "null") {
                        return;
                    }
                    if !inferred_set.contains(col_name) {
                        out.push(Diagnostic {
                            severity: DiagnosticSeverity::Error,
                            message: format!(
                                "Column `{}` is not available in the splice context for `{}` \
                                 in `{}`",
                                col_name, param.name, sig.name
                            ),
                            range: to_range(col_range, text),
                            code: Some(DiagnosticCode::FragmentColumnMissing),
                            data: None,
                        });
                    }
                },
            );
        }
    }

    out
}

/// Check that a Tier 3 function's body synthesises a return type that is
/// compatible with the declared `-> Expr<T>` annotation.
///
/// Only `Expr<T>` return annotations are checked — `TableExpr`, `SelectItems`,
/// and other sorts are deferred to later phases. For non-`Expr<T>` annotations
/// this function returns an empty vec (skip the check).
///
/// Returns a single `ReturnTypeMismatch` diagnostic anchored at the body
/// expression when the synthesised type doesn't satisfy the declared
/// constraint.
///
/// Pure — no Salsa dependency.
pub fn check_tier3_return_type(sig: &FunctionSig, body: &Expr, text: &str) -> Vec<Diagnostic> {
    use smelt_types::signatures::Tier;

    // Only Tier 3 has a declared return type.
    if sig.tier != Tier::Three {
        return Vec::new();
    }

    // Extract the declared return constraint — only handle `Expr<T>` here.
    // Skip `TableExpr`, `SelectItems`, and any parse error / absent annotation.
    let declared_constraint = match &sig.return_type {
        Some(Ok(SmeltType::Expr(constraint))) => constraint.clone(),
        _ => return Vec::new(), // non-Expr<T> or missing — skip
    };

    // Build a seeded param context and infer the body's return type.
    // Phase 27: Set `expected_return` on the context when we have a concrete
    // declared return type (e.g. `-> Expr<Double>`). This allows built-in
    // generics like `COALESCE` to widen their type-variable binding to the
    // expected return type (§16 #14 Decision 14).
    let mut ctx = seed_param_context(&sig.params);
    if let TypeConstraint::Concrete(ref dt) = declared_constraint {
        ctx.expected_return = Some(dt.clone());
    }
    let inferred = match infer_expression_type(body, &ctx) {
        Some(tc) => tc.data_type,
        None => return Vec::new(), // can't infer — skip
    };

    // Unknown / Null bodies can't be verified.
    if matches!(inferred, DataType::Unknown | DataType::Null) {
        return Vec::new();
    }

    // Check whether the inferred type satisfies the declared constraint.
    if declared_constraint.satisfies(&inferred) {
        return Vec::new();
    }

    // Mismatch — anchor at the body expression.
    let body_range = to_range(body.text_range(), text);
    let declared_text = sig
        .return_type_text
        .as_deref()
        .unwrap_or("<unknown return type>");
    vec![Diagnostic {
        severity: DiagnosticSeverity::Error,
        message: format!(
            "Return type mismatch: declared `-> {}` but body evaluates to `{}`",
            declared_text, inferred
        ),
        range: body_range,
        code: Some(DiagnosticCode::ReturnTypeMismatch),
        data: None,
    }]
}

/// For a Tier 3 function, return the declared return type as a hover-ready
/// string (e.g. `"-> Expr<Double>"`). Returns `None` for Tier 1/2 or when
/// the return annotation is unparseable or absent.
///
/// Pure — no Salsa dependency.
pub fn declared_return_hover_text(sig: &FunctionSig) -> Option<String> {
    use smelt_types::signatures::Tier;
    if sig.tier != Tier::Three {
        return None;
    }
    sig.return_type_text.as_deref().map(|t| format!("-> {t}"))
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
