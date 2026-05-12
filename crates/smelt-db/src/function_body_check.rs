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
use smelt_parser::ast::{
    BinaryExpr, Cte, Expr, FunctionCall, Lambda, SelectStmt, SmeltPathCall, TableRef,
};
use smelt_parser::offset_to_position;
use smelt_types::signatures::{
    check_schema_requirement, column_ref_field, unify_call, ContextRef, ExprKind, FrameInfo,
    FunctionSig, ParamSpec, SchemaMismatch, SchemaRequirement, Signature, SmeltType, StructRowTail,
    Tier, TypeConstraint, UnificationError,
};
use smelt_types::{DataType, TypedColumn};
use std::path::PathBuf;

use crate::schema::{Column, ColumnSource, ModelSchema};
use crate::type_inference::{
    check_column_ref_field_diagnostics, check_meta_text_lift_diagnostics,
    check_model_ref_source_ref_field_diagnostics, check_undeclared_columns,
    check_wide_reflection_diagnostics, infer_cte_columns, infer_expression_kind,
    infer_expression_type, walk_expression_columns_with_visitor, walk_select_columns_with_visitor,
    TypeContext,
};
use crate::{Diagnostic, DiagnosticCode, DiagnosticData, DiagnosticSeverity, Range};

/// Discriminates the three type-checking tiers (§8) inside
/// `check_function_body` and `check_smelt_path_call`.
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

/// Callback type for dispatching nested `smelt.functions.*` path-form calls
/// encountered during body-recursion (Phase 5a).
pub type NestedPathCallHandler<'a> =
    dyn Fn(&SmeltPathCall, &TypeContext, &str) -> Vec<Diagnostic> + 'a;

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
/// `smelt.functions.*` path-form calls through `nested` so frames stack up
/// across arbitrary expansion depth.
///
/// Callers pass a closure that invokes [`check_smelt_path_call`] on each
/// nested call with the re-bound context. The plain
/// [`check_function_body`] entrypoint delegates here with `None` and
/// therefore matches its pre-Phase-12 behaviour exactly.
pub fn check_function_body_with_expansion(
    sig: &FunctionSig,
    body: &Expr,
    text: &str,
    nested: &NestedPathCallHandler<'_>,
) -> Vec<Diagnostic> {
    check_function_body_inner(sig, body, text, Some(nested))
}

/// Phase-26 variant: walk a function body using a *caller-supplied* `TypeContext`
/// rather than re-building one from the signature's parameter annotations.
///
/// Used by [`check_smelt_path_call`] when expanding a body at a call site —
/// the call-site-bound `body_ctx` already has concrete argument types, so we skip
/// `seed_param_context` and use it directly. Duplicate-parameter checks are also
/// skipped (they already ran at definition time via `function_body_diagnostics_for_file`).
pub fn walk_body_with_ctx(
    body: &Expr,
    ctx: &TypeContext,
    text: &str,
    nested: &NestedPathCallHandler<'_>,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    walk_body(body, ctx, text, &mut diagnostics, Some(nested));
    diagnostics
}

/// Phase B (meta-language): walk a HOF lambda body for type errors and stamp
/// each resulting diagnostic with an **anonymous expansion frame** whose
/// `fn_id = None` and `function = "<hof>"` (angle-bracketed form per spec).
///
/// This is the HOF equivalent of `check_function_body_with_expansion`: whereas
/// `smelt.define` expansions produce named frames (`fn_id = Some(sig.name)`),
/// HOF expansions produce anonymous frames because the HOF has no user-declared
/// function body — it is a built-in operator.
///
/// The frame stack entry:
/// - `function` = `"<hof>"` (e.g. `"<map>"`) — angle brackets distinguish
///   synthesised HOF frames from user-named function frames (spec §FrameInfo).
/// - `fn_id` = `None` (anonymous — no declaring file)
/// - `call_site_range` = `call_range` (span of the outer HOF call expression)
/// - `param`, `bound_type`, `decl_path`, `decl_range`, `element_index` = defaults
///
/// `nested` propagates the `smelt.functions.*` dispatch handler so that
/// `smelt.define`-defined function calls inside a HOF lambda body still
/// receive expansion-frame stamping (multi-frame chain support).
///
/// Pure function — no Salsa dependency.
pub fn walk_hof_lambda_body_with_anonymous_frame(
    lambda_body: &Expr,
    lambda_ctx: &TypeContext,
    text: &str,
    hof_name: &str,
    call_range: Option<Range>,
    nested: Option<&NestedPathCallHandler<'_>>,
) -> Vec<Diagnostic> {
    let mut inner_diags = Vec::new();
    walk_body(lambda_body, lambda_ctx, text, &mut inner_diags, nested);

    // Stamp every inner diagnostic with an anonymous HOF expansion frame.
    if inner_diags.is_empty() {
        return inner_diags;
    }

    let frame = FrameInfo {
        function: format!("<{}>", hof_name.to_lowercase()),
        param: String::new(),
        bound_type: String::new(),
        decl_path: None,
        decl_range: None,
        call_site_range: call_range,
        fn_id: None, // anonymous — HOF has no declaring file
        element_index: None,
        column_origin: None,
    };

    inner_diags
        .into_iter()
        .map(|mut d| {
            let frames = match d.data.take() {
                Some(DiagnosticData::ExpansionFrames(mut existing)) => {
                    // Prepend the anonymous HOF frame (innermost-first ordering).
                    existing.insert(0, frame.clone());
                    existing
                }
                _ => vec![frame.clone()],
            };
            d.data = Some(DiagnosticData::ExpansionFrames(frames));
            d
        })
        .collect()
}

/// Phase C (meta-language) variant of [`walk_hof_lambda_body_with_anonymous_frame`]
/// that accepts an explicit `column_origin` to stamp onto the anonymous HOF frame.
///
/// When the HOF source list comes from `smelt.columns_of(t)`, callers can pass the
/// column's source span (from the upstream `ModelSchema`) so the frame's
/// `column_origin` field carries that span. When the source was not a
/// `smelt.columns_of` call (or the span is not statically resolvable), pass `None`.
///
/// Pure function — no Salsa dependency.
pub fn walk_hof_lambda_body_with_anonymous_frame_and_origin(
    lambda_body: &Expr,
    lambda_ctx: &TypeContext,
    text: &str,
    hof_name: &str,
    call_range: Option<Range>,
    nested: Option<&NestedPathCallHandler<'_>>,
    column_origin: Option<TextRange>,
) -> Vec<Diagnostic> {
    let mut inner_diags = Vec::new();
    walk_body(lambda_body, lambda_ctx, text, &mut inner_diags, nested);

    // Stamp every inner diagnostic with an anonymous HOF expansion frame,
    // including the Phase C `column_origin` extension.
    if inner_diags.is_empty() {
        return inner_diags;
    }

    let frame = FrameInfo {
        function: format!("<{}>", hof_name.to_lowercase()),
        param: String::new(),
        bound_type: String::new(),
        decl_path: None,
        decl_range: None,
        call_site_range: call_range,
        fn_id: None, // anonymous — HOF has no declaring file
        element_index: None,
        column_origin,
    };

    inner_diags
        .into_iter()
        .map(|mut d| {
            let frames = match d.data.take() {
                Some(DiagnosticData::ExpansionFrames(mut existing)) => {
                    // Prepend the anonymous HOF frame (innermost-first ordering).
                    existing.insert(0, frame.clone());
                    existing
                }
                _ => vec![frame.clone()],
            };
            d.data = Some(DiagnosticData::ExpansionFrames(frames));
            d
        })
        .collect()
}

fn check_function_body_inner(
    sig: &FunctionSig,
    body: &Expr,
    text: &str,
    nested: Option<&NestedPathCallHandler<'_>>,
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
pub(crate) fn seed_param_context(params: &[ParamSpec]) -> TypeContext {
    let mut ctx = TypeContext::new();
    for p in params {
        if p.name.is_empty() {
            continue;
        }
        let bound_type = param_binding_type(p);
        ctx.add_function_param(&p.name, TypedColumn::nullable(bound_type));
        // Phase 44b: record SelectItems<Kind> parameters so that bare
        // references to these params inside PASSING bodies inherit the
        // declared kind (see `infer_expression_kind`'s fragment_param_kinds
        // lookup).
        if let Some(Ok(SmeltType::SelectItems { kind, .. })) = &p.type_ref {
            ctx.add_fragment_param_kind(&p.name, *kind);
        }
        // Carry-over (type-expert review): record the full SmeltType for
        // List<T> (and any non-trivial meta-language type) parameters so that
        // HOF inference can recover `SmeltType::List(...)` rather than
        // collapsing to `DataType::Unknown`.  The scalar projection stored in
        // `function_params` is insufficient for HOF first-argument inference.
        //
        // Phase C: also register `ColumnRef` params so that
        // `is_meta_text_value` (called from `check_meta_text_lift_diagnostics`)
        // can recognise `<param>.name` as a meta-Text lift and
        // `check_function_select_body` can suppress the spurious
        // `UnknownIdentifier` that `check_undeclared_columns` would otherwise
        // emit for the qualified reference `<param>.name` before the lift check
        // runs.
        if let Some(Ok(smelt_ty)) = &p.type_ref {
            if matches!(
                smelt_ty,
                SmeltType::List(_)
                    | SmeltType::Unknown
                    | SmeltType::ColumnRef
                    | SmeltType::ModelRef
                    | SmeltType::SourceRef
            ) {
                ctx.add_function_param_smelt_type(&p.name, smelt_ty.clone());
            }
        }
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
        // `Struct<{…}>` (Phase 35) params — runtime type unknown until Phase 36.
        Some(Ok(SmeltType::Struct { .. })) => DataType::Unknown,
        // `List<T>` and `Unknown` (Phase A meta-language) — compile-time only; no
        // runtime DataType equivalent in Phase A.
        Some(Ok(SmeltType::List(_))) | Some(Ok(SmeltType::Unknown)) => DataType::Unknown,
        // `Lambda<T, U>` (Phase B meta-language) — meta-only; not a valid parameter sort.
        Some(Ok(SmeltType::Lambda(_, _))) => DataType::Unknown,
        // `ColumnRef` (Phase C meta-language) — meta-only; not a SQL DataType.
        Some(Ok(SmeltType::ColumnRef)) => DataType::Unknown,
        // `ModelRef` / `SourceRef` (Phase D meta-language) — meta-only; not a SQL DataType.
        Some(Ok(SmeltType::ModelRef)) | Some(Ok(SmeltType::SourceRef)) => DataType::Unknown,
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

/// Is this parameter an `Expr<Struct<{…}>>` sort?
///
/// Phase 36 treats struct parameters similarly to `TableExpr` parameters:
/// the caller-supplied column schema is consulted at call-site expansion
/// to unify the declared field requirements against the concrete argument.
pub(crate) fn is_struct_param(p: &ParamSpec) -> bool {
    matches!(&p.type_ref, Some(Ok(SmeltType::Struct { .. })))
}

/// Phase 46: is this argument expression *clearly* not a table?
///
/// Returns `true` for the unambiguously-not-a-table cases that should
/// surface a diagnostic at the argument span when passed to a
/// `TableExpr` parameter. We deliberately scope this down to bare
/// literals (number / string / boolean / NULL) — anything more
/// exotic (binary expressions, function calls, etc.) might be a
/// pattern Phase 47 widens to support, so we don't preemptively emit
/// diagnostics for those.
fn is_clearly_non_table(arg_expr: &Expr) -> bool {
    use smelt_parser::syntax_kind::{SyntaxKind as Sk, SyntaxToken};

    // Walk the argument expression's tokens; if all non-trivia tokens
    // are a single literal (NUMBER, STRING, TRUE/FALSE, NULL), treat
    // the argument as a literal.
    let mut content_tokens: Vec<SyntaxToken> = Vec::new();
    for tok in arg_expr.syntax().descendants_with_tokens() {
        if let Some(t) = tok.into_token() {
            match t.kind() {
                Sk::WHITESPACE | Sk::COMMENT => continue,
                _ => content_tokens.push(t),
            }
        }
    }
    if content_tokens.len() != 1 {
        return false;
    }
    matches!(
        content_tokens[0].kind(),
        Sk::NUMBER | Sk::STRING | Sk::NULL_KW
    )
}

/// Extract the declared fields and tail from a `Struct<{…}>` parameter.
///
/// Returns `None` for non-struct parameters (shouldn't happen if the caller
/// already checked [`is_struct_param`]).
pub(crate) fn struct_param_fields(
    p: &ParamSpec,
) -> Option<(&Vec<(String, DataType)>, &StructRowTail)> {
    match &p.type_ref {
        Some(Ok(SmeltType::Struct { fields, tail })) => Some((fields, tail)),
        _ => None,
    }
}

/// Pure struct-field unification (Phase 36).
///
/// Given the declared fields of an `Expr<Struct<{…}>>` parameter and the
/// concrete fields of the argument expression, verify that every declared
/// field is present and type-compatible.
///
/// Returns:
/// - `Ok(Some(extras))` when the tail is `Named` and there are extra fields.
/// - `Ok(None)` when the requirement is satisfied with no named tail, or
///   when the tail is `Named` but there are no extras.
/// - `Err(mismatch)` when a declared field is missing or has an incompatible
///   type in the concrete argument.
///
/// Uses the same [`SchemaMismatch`] type as `check_schema_requirement` so
/// `row_requirement_diagnostic` can render the error message identically.
///
/// Pure — no Salsa, no I/O.
pub fn check_struct_row_var_binding(
    declared_fields: &[(String, DataType)],
    concrete_fields: &[(String, DataType)],
    tail: &StructRowTail,
) -> Result<Option<Vec<(String, DataType)>>, SchemaMismatch> {
    use smelt_types::signatures::DataTypeReq;

    // 1. Every declared field must be present in the concrete set with a
    //    compatible type.
    for (field_name, required_type) in declared_fields {
        let Some((_, actual_dt)) = concrete_fields.iter().find(|(n, _)| n == field_name) else {
            return Err(SchemaMismatch::MissingColumn {
                column: field_name.clone(),
                required: DataTypeReq::Concrete(required_type.clone()),
            });
        };
        // Use the same assignment-compatibility check used elsewhere in
        // the checker. We only care about exact-type or widening here:
        // `Timestamp` must be `Timestamp`, `Integer` may widen to `BigInt`.
        if !types_assignment_compatible(required_type, actual_dt) {
            return Err(SchemaMismatch::TypeMismatch {
                column: field_name.clone(),
                required: DataTypeReq::Concrete(required_type.clone()),
                actual: actual_dt.to_string(),
            });
        }
    }

    // 2. Collect extras (concrete fields not in declared).
    let declared_names: std::collections::HashSet<&str> =
        declared_fields.iter().map(|(n, _)| n.as_str()).collect();
    let extras: Vec<(String, DataType)> = concrete_fields
        .iter()
        .filter(|(n, _)| !declared_names.contains(n.as_str()))
        .cloned()
        .collect();

    match tail {
        StructRowTail::None | StructRowTail::Anon => Ok(None),
        StructRowTail::Named(_name) => Ok(Some(extras)),
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
            "Argument for parameter `{}` of `smelt.functions.{}` is missing required column `{}: {}`",
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
            "Column `{}` in argument for parameter `{}` of `smelt.functions.{}` has type `{}`, expected `{}`",
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

/// Build a [`Diagnostic`] describing a struct field-requirement failure
/// (Phase 36).
///
/// Message shape: "Struct argument for `<param>` of `smelt.fn.<fn>` is missing
/// field `<col>: <type>`" — mirrors the TableExpr wording but uses "field"
/// instead of "column" to match struct terminology.
fn struct_field_diagnostic(
    mismatch: &SchemaMismatch,
    fn_name: &str,
    param_name: &str,
    arg_range: Range,
) -> Diagnostic {
    let message = match mismatch {
        SchemaMismatch::MissingColumn { column, required } => format!(
            "Struct argument for `{}` of `smelt.functions.{}` is missing field `{}: {}`",
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
            "Field `{}` in struct argument for `{}` of `smelt.functions.{}` has type `{}`, expected `{}`",
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
/// `smelt.functions.*` path-form calls through [`check_smelt_path_call`].
/// When `Some`, every nested `SMELT_PATH_CALL` encountered in the body is
/// checked via the closure so frames stack up across arbitrary expansion
/// depth. When `None` the walker matches its pre-Phase-12 behaviour —
/// useful for unit tests and the [`check_function_body`] legacy entry.
fn walk_body(
    expr: &Expr,
    ctx: &TypeContext,
    text: &str,
    out: &mut Vec<Diagnostic>,
    nested: Option<&NestedPathCallHandler<'_>>,
) -> Option<TypedColumn> {
    // Phase 12: nested `smelt.functions.*` path-form call — dispatch through
    // the handler so the call-site checker recurses with the caller's
    // bindings visible. The handler emits any call-site diagnostics (and
    // body-cascade diagnostics with `ExpansionFrames`). We still fall
    // through to generic inference for the return type below.
    if let (Some(call), Some(nested)) = (expr.as_smelt_path_call(), nested) {
        let nested_diags = nested(&call, ctx, text);
        out.extend(nested_diags);
        // Still compute a return type via the inference engine — when the
        // call checks clean, the inferred type flows up to the parent
        // expression (e.g. a binary-op) for further type checking.
        return infer_expression_type(expr, ctx);
    }

    // Phase 38: smelt.as_struct(alias [EXCEPT cols]) — do not recurse into
    // children. The alias inside is a table qualifier, not a column expression,
    // so recursing would flag it as `UnknownIdentifier`. Delegate directly to
    // type inference for the return type.
    if expr.as_smelt_as_struct_call().is_some() {
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
    nested: Option<&NestedPathCallHandler<'_>>,
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

/// Check a single `smelt.functions.*` (path-form) call site.
///
/// Produces diagnostics:  `ArgTypeMismatch`, `MissingArgument`, `UnknownSmeltFn`,
/// body-type errors, and `RowRequirementUnsatisfied`.
///
/// Pure-function-rule: no Salsa calls; all Salsa boundary closures are passed in.
#[allow(
    clippy::type_complexity,
    clippy::too_many_arguments,
    clippy::only_used_in_recursion
)]
pub fn check_smelt_path_call(
    call: &SmeltPathCall,
    ctx: &TypeContext,
    text: &str,
    sig_lookup: &dyn Fn(&str) -> Option<FunctionSig>,
    builtin_lookup: &dyn Fn(&str) -> Option<&'static Signature>,
    lub: &dyn Fn(&DataType, &DataType) -> DataType,
    body_lookup: &dyn Fn(&FunctionSig) -> Option<(String, BodyShape)>,
    decl_lookup: &dyn Fn(&FunctionSig) -> Option<PathBuf>,
    tableexpr_schema_lookup: &dyn Fn(&Expr, &TypeContext) -> Option<Vec<(String, TypedColumn)>>,
    default_type_lookup: &dyn Fn(&FunctionSig, &str) -> Option<DataType>,
    table_ref_schema_lookup: &dyn Fn(&TableRef) -> Option<Vec<(String, TypedColumn)>>,
    smelt_path_schema_lookup: &dyn Fn(&SmeltPathCall) -> Option<Vec<(String, TypedColumn)>>,
    // TB-5: validates that the call's directory-segment prefix matches the
    // workspace-relative directory of the file declaring `name`. Returns
    // `true` iff a function with that name exists in the file at exactly
    // that workspace-relative directory. Used to emit `UnknownSmeltFn` when
    // the leaf name resolves but the path prefix is wrong (e.g. includes
    // the file stem, or names a directory the function does not live in).
    path_prefix_validator: &dyn Fn(&[String], &str) -> bool,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    // 1. Resolve the function. Segments after the leading `smelt` are returned
    //    by `call.segments()` — for `smelt.functions.foo` that is
    //    `["functions", "foo"]`. The leaf name is used for lookup; the full
    //    path is used in diagnostic messages.
    let path_range = match call.call_path_range() {
        Some(r) => to_range(r, text),
        None => to_range(call.text_range(), text),
    };
    let segments = call.segments();
    let Some(name) = segments.last().cloned() else {
        return diagnostics;
    };

    // Phase C: skip smelt.columns_of — it is a meta-builtin handled by
    // `check_columns_of_diagnostics` and the `ColumnsOfUnresolvableSchema`
    // wiring block in `check_file_diagnostics`. It does not go through the
    // smelt-function call dispatcher and has no `FunctionSig` in the registry.
    if segments.len() == 1 && name.to_lowercase() == "columns_of" {
        return diagnostics;
    }

    // Build the display path for messages: "smelt.functions.foo"
    let display_path = format!("smelt.{}", segments.join("."));

    let Some(sig) = sig_lookup(&name) else {
        if let Some(builtin_sig) = builtin_lookup(&name) {
            // Builtin function: collect args and run unify_call.
            let arg_list_ref = call.arg_list();
            let positional_exprs: Vec<Expr> = arg_list_ref
                .as_ref()
                .map(|al| al.positional_args())
                .unwrap_or_default();
            let named_values: Vec<Expr> = arg_list_ref
                .as_ref()
                .map(|al| al.named_params().filter_map(|np| np.value_expr()).collect())
                .unwrap_or_default();
            let mut arg_exprs: Vec<Expr> =
                Vec::with_capacity(positional_exprs.len() + named_values.len());
            arg_exprs.extend(positional_exprs);
            arg_exprs.extend(named_values);

            let arg_types: Vec<DataType> = arg_exprs
                .iter()
                .map(|a| {
                    infer_expression_type(a, ctx)
                        .map(|t| t.data_type)
                        .unwrap_or(DataType::Unknown)
                })
                .collect();

            match unify_call(builtin_sig, &arg_types, lub) {
                Ok(_) => {}
                Err(UnificationError::MissingArgs { expected, got }) => {
                    diagnostics.push(Diagnostic {
                        severity: DiagnosticSeverity::Error,
                        message: format!(
                            "`{}` expects at least {} argument(s), got {}",
                            display_path, expected, got
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
                    let arg_range = arg_exprs
                        .get(position.saturating_sub(1))
                        .map(|e| to_range(e.text_range(), text))
                        .unwrap_or(path_range);
                    diagnostics.push(Diagnostic {
                        severity: DiagnosticSeverity::Error,
                        message: format!(
                            "Argument at position {} has type `{}`, which does not satisfy constraint `{}` of `{}`",
                            position,
                            actual,
                            format_constraint(&param_constraint),
                            display_path
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
                            "Type variable `{}` in `{}` inferred inconsistently across positions {:?}: {}",
                            var_name, display_path, positions, type_list
                        ),
                        range: arg_range,
                        code: Some(DiagnosticCode::ArgTypeMismatch),
                        data: None,
                    });
                }
                Err(UnificationError::EmptyVariadicTypeVar { .. }) => {
                    diagnostics.push(Diagnostic {
                        severity: DiagnosticSeverity::Error,
                        message: format!(
                            "`{}` is variadic and requires at least one argument",
                            display_path
                        ),
                        range: path_range,
                        code: Some(DiagnosticCode::MissingArgument),
                        data: None,
                    });
                }
                Err(UnificationError::TooManyArgs { .. }) => {}
            }
        } else {
            diagnostics.push(Diagnostic {
                severity: DiagnosticSeverity::Error,
                message: format!("Unknown function `{}`", display_path),
                range: path_range,
                code: Some(DiagnosticCode::UnknownSmeltFn),
                data: None,
            });
        }
        return diagnostics;
    };

    // TB-5: enforce the call-path prefix. The leaf-name lookup succeeded,
    // but the directory segments (everything before the leaf) must equal
    // the workspace-relative directory of the declaring file. Without this
    // check, paths like `smelt.functions.status.is_shipped` (file stem in
    // the path) and `smelt.functions.nonexistent.is_shipped` would resolve
    // because we only matched on the leaf name. Builtins are not subject
    // to this rule (they have no declaring file); they were already
    // dispatched through the `builtin_lookup` branch above.
    let dir_segments: Vec<String> = if segments.is_empty() {
        Vec::new()
    } else {
        segments[..segments.len() - 1].to_vec()
    };
    if !path_prefix_validator(&dir_segments, &name) {
        diagnostics.push(Diagnostic {
            severity: DiagnosticSeverity::Error,
            message: format!("Unknown function `{}`", display_path),
            range: path_range,
            code: Some(DiagnosticCode::UnknownSmeltFn),
            data: None,
        });
        return diagnostics;
    }

    // 2. Bind arguments to parameters.
    let arg_list = call.arg_list();
    let positional: Vec<Expr> = arg_list
        .as_ref()
        .map(|al| al.positional_args())
        .unwrap_or_default();
    let named: Vec<smelt_parser::ast::NamedParam> = arg_list
        .as_ref()
        .map(|al| al.named_params().collect())
        .unwrap_or_default();

    let mut bindings: std::collections::HashMap<String, (Expr, TextRange)> =
        std::collections::HashMap::new();

    for (i, arg) in positional.iter().enumerate() {
        if let Some(p) = sig.params.get(i) {
            bindings.insert(p.name.clone(), (arg.clone(), arg.text_range()));
        }
    }

    for np in &named {
        let Some(nm) = np.name() else { continue };
        if let Some(value_expr) = np.value_expr() {
            bindings.insert(nm.clone(), (value_expr.clone(), value_expr.text_range()));
        }
    }

    // PASSING clauses.
    let passing_clauses: Vec<smelt_parser::ast::PassingClause> = call.passing_clauses().collect();
    for clause in &passing_clauses {
        let Some(clause_name) = clause.name() else {
            continue;
        };
        let param_exists = sig.params.iter().any(|p| p.name == clause_name);
        if !param_exists {
            let clause_range = clause
                .name_range()
                .map(|r| to_range(r, text))
                .unwrap_or(path_range);
            diagnostics.push(Diagnostic {
                severity: DiagnosticSeverity::Error,
                message: format!(
                    "PASSING clause names `{}` which is not a parameter of `{}`",
                    clause_name, display_path
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
    let mut frame_bindings: Vec<(String, String)> = Vec::new();

    for param in &sig.params {
        if param.name.is_empty() {
            continue;
        }

        if is_tableexpr_param(param) {
            if let Some((arg_expr, arg_range)) = bindings.get(&param.name) {
                let cols = tableexpr_schema_lookup(arg_expr, ctx).unwrap_or_default();

                if cols.is_empty() && is_clearly_non_table(arg_expr) {
                    diagnostics.push(Diagnostic {
                        severity: DiagnosticSeverity::Error,
                        message: format!(
                            "Argument for TableExpr parameter `{}` of `{}` must be a table reference, CTE, or subquery — got `{}`",
                            param.name,
                            display_path,
                            arg_expr.text().to_string().trim()
                        ),
                        range: to_range(*arg_range, text),
                        code: Some(DiagnosticCode::ArgTypeMismatch),
                        data: None,
                    });
                    frame_bindings.push((param.name.clone(), "<not-a-table>".to_string()));
                    continue;
                }

                if let Some(req) = tableexpr_schema_requirement(param) {
                    let arg_schema: Vec<(String, DataType)> = cols
                        .iter()
                        .map(|(n, tc)| (n.clone(), tc.data_type.clone()))
                        .collect();
                    match check_schema_requirement(req, &arg_schema) {
                        Ok(binding) => {
                            body_ctx.add_tableexpr_param(&param.name, &cols);
                            if let Some(b) = binding {
                                body_ctx.set_row_var_binding(&b.name, b.extras);
                            }
                            frame_bindings.push((param.name.clone(), "TableExpr".to_string()));
                        }
                        Err(mismatch) => {
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
                    body_ctx.add_tableexpr_param(&param.name, &cols);
                    frame_bindings.push((param.name.clone(), "TableExpr".to_string()));
                }
            } else if !param.has_default {
                diagnostics.push(Diagnostic {
                    severity: DiagnosticSeverity::Error,
                    message: format!(
                        "Missing required argument `{}` for `{}`",
                        param.name, display_path
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

        if is_struct_param(param) {
            if let Some((arg_expr, arg_range)) = bindings.get(&param.name) {
                let cols = tableexpr_schema_lookup(arg_expr, ctx)
                    .or_else(|| {
                        let qualifier = arg_expr.text().trim().to_string();
                        if qualifier.is_empty() {
                            return None;
                        }
                        let cols: Vec<(String, TypedColumn)> = ctx
                            .columns_for_qualifier(&qualifier)
                            .into_iter()
                            .map(|(col_name, tc)| (col_name.to_string(), tc.clone()))
                            .collect();
                        if cols.is_empty() {
                            None
                        } else {
                            Some(cols)
                        }
                    })
                    .unwrap_or_default();
                let concrete_fields: Vec<(String, DataType)> = cols
                    .iter()
                    .map(|(n, tc)| (n.clone(), tc.data_type.clone()))
                    .collect();

                if let Some((declared_fields, tail)) = struct_param_fields(param) {
                    if concrete_fields.is_empty() {
                        body_ctx.add_function_param(
                            &param.name,
                            TypedColumn::nullable(DataType::Unknown),
                        );
                        frame_bindings.push((param.name.clone(), "Struct<unknown>".to_string()));
                    } else {
                        match check_struct_row_var_binding(declared_fields, &concrete_fields, tail)
                        {
                            Ok(extras_opt) => {
                                body_ctx.add_function_param(
                                    &param.name,
                                    TypedColumn::nullable(DataType::Unknown),
                                );
                                if let (StructRowTail::Named(var_name), Some(extras)) =
                                    (tail, extras_opt)
                                {
                                    body_ctx.set_row_var_binding(var_name, extras);
                                }
                                frame_bindings.push((
                                    param.name.clone(),
                                    format!("Struct<{} fields>", concrete_fields.len()),
                                ));
                            }
                            Err(mismatch) => {
                                diagnostics.push(struct_field_diagnostic(
                                    &mismatch,
                                    &sig.name,
                                    &param.name,
                                    to_range(*arg_range, text),
                                ));
                                frame_bindings
                                    .push((param.name.clone(), "<struct-req-failed>".to_string()));
                            }
                        }
                    }
                } else {
                    body_ctx
                        .add_function_param(&param.name, TypedColumn::nullable(DataType::Unknown));
                    frame_bindings.push((param.name.clone(), "Struct<?>".to_string()));
                }
            } else if !param.has_default {
                diagnostics.push(Diagnostic {
                    severity: DiagnosticSeverity::Error,
                    message: format!(
                        "Missing required argument `{}` for `{}`",
                        param.name, display_path
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
                let arg_type = infer_expression_type(arg_expr, ctx)
                    .map(|t| t.data_type)
                    .unwrap_or(DataType::Unknown);

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
                            "Missing required argument `{}` for `{}`",
                            param.name, display_path
                        ),
                        range: path_range,
                        code: Some(DiagnosticCode::MissingArgument),
                        data: None,
                    });
                    body_ctx
                        .add_function_param(&param.name, TypedColumn::nullable(DataType::Unknown));
                    frame_bindings.push((param.name.clone(), "<missing>".to_string()));
                } else {
                    let dt = default_type_lookup(&sig, &param.name).unwrap_or(DataType::Unknown);
                    body_ctx.add_function_param(&param.name, TypedColumn::nullable(dt.clone()));
                    frame_bindings.push((param.name.clone(), format!("{} (default)", dt)));
                }
            }
        }
    }

    // Register SelectItems fragment-param kinds.
    for p in &sig.params {
        if p.name.is_empty() {
            continue;
        }
        if let Some(Ok(SmeltType::SelectItems { kind, .. })) = &p.type_ref {
            body_ctx.add_fragment_param_kind(&p.name, *kind);
        }
    }

    // Pre-resolve JOIN-aliased schemas for shadow-warning check.
    let preview_body_for_joins = body_lookup(&sig);
    if let Some((_, BodyShape::Select(preview_select))) = &preview_body_for_joins {
        register_join_alias_schemas(&mut body_ctx, preview_select, &sig, table_ref_schema_lookup);
    }

    diagnostics.extend(compute_shadow_warnings(&sig, &body_ctx));

    let shadow_warnings: Vec<Diagnostic> = diagnostics
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::ParameterShadowsColumn))
        .cloned()
        .collect();
    diagnostics.retain(|d| d.code != Some(DiagnosticCode::ParameterShadowsColumn));

    if !diagnostics.is_empty() {
        diagnostics.extend(shadow_warnings);
        return diagnostics;
    }

    let has_schema_param = sig.params.iter().any(|p| {
        matches!(
            &p.type_ref,
            Some(Ok(SmeltType::TableExpr(_)))
                | Some(Ok(SmeltType::SelectItems { .. }))
                | Some(Ok(SmeltType::Struct { .. }))
        )
    });
    if sig.tier != Tier::One && !has_schema_param {
        diagnostics.extend(shadow_warnings);
        return diagnostics;
    }

    let Some((body_text, body_shape)) = body_lookup(&sig) else {
        diagnostics.extend(shadow_warnings);
        return diagnostics;
    };

    // Nested handler for SMELT_PATH_CALL nodes inside the body (recursive).
    let nested_path_handler = |nested_call: &SmeltPathCall,
                               nested_ctx: &TypeContext,
                               nested_text: &str|
     -> Vec<Diagnostic> {
        check_smelt_path_call(
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
            table_ref_schema_lookup,
            smelt_path_schema_lookup,
            path_prefix_validator,
        )
    };

    let inner = match &body_shape {
        BodyShape::Expression(body_expr) => {
            walk_body_with_ctx(body_expr, &body_ctx, &body_text, &nested_path_handler)
        }
        BodyShape::Select(select_stmt) => {
            for (name, sig_entry) in ctx.function_signatures_iter() {
                body_ctx.add_function_signature(name, sig_entry.clone());
            }

            let (body_ctx_with_ctes, _cycle_diags) =
                extract_function_body_cte_schemas(select_stmt, &body_ctx, &body_text);
            let body_ctx = body_ctx_with_ctes;

            let mut body_diags = check_function_select_body(
                &sig,
                select_stmt,
                &body_text,
                &body_ctx,
                &nested_path_handler,
                None,
            );
            body_diags.extend(check_fragment_context_bindings(
                &sig,
                select_stmt,
                &body_ctx,
                ctx,
                &bindings,
                text,
            ));
            body_diags
        }
    };

    let decl_path = decl_lookup(&sig);
    let decl_range = Some(sig.name_range);
    let call_site_range = Some(path_range);

    for mut d in inner {
        d.range = path_range;

        let mut frames: Vec<FrameInfo> = match d.data.take() {
            Some(DiagnosticData::ExpansionFrames(existing)) => existing,
            _ => Vec::new(),
        };

        if !frame_bindings.is_empty() {
            frames.push(FrameInfo {
                function: sig.name.clone(),
                param: frame_bindings
                    .iter()
                    .map(|(p, t)| format!("{}: {}", p, t))
                    .collect::<Vec<_>>()
                    .join(", "),
                bound_type: String::new(),
                decl_path: decl_path.clone(),
                decl_range,
                call_site_range,
                fn_id: Some(sig.name.clone()),
                element_index: None,
                column_origin: None,
            });
        } else {
            frames.push(FrameInfo {
                function: sig.name.clone(),
                param: String::new(),
                bound_type: String::new(),
                decl_path: decl_path.clone(),
                decl_range,
                call_site_range,
                fn_id: Some(sig.name.clone()),
                element_index: None,
                column_origin: None,
            });
        }
        d.data = Some(DiagnosticData::ExpansionFrames(frames));
        diagnostics.push(d);
    }

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

/// Phase 45: walk a SELECT-shaped function body's FROM clause and
/// JOIN clauses, and register every aliased table-ref's column schema
/// into `body_ctx` so the body's bare-column resolver can see joined
/// schemas (review findings #10 and #21).
///
/// Specifically:
///   - For each `TableRef` in the FROM clause's comma-separated list:
///     if the ref is a bare identifier matching a TableExpr parameter
///     name *and* has an alias, register the alias as a synonym for
///     the parameter via `body_ctx.add_alias`. (Other cases — refs
///     resolved by `table_ref_schema_lookup` — are seeded as
///     full alias schemas via `add_tableexpr_param`.)
///   - For each `JoinClause`: get its `TableRef`, look up its schema
///     via `table_ref_schema_lookup`, and seed it as a TableExpr-param-
///     style alias.
///
/// Subqueries / CTE references / derived tables in JOIN/FROM
/// position are deferred (`table_ref_schema_lookup` returns `None`
/// for them). Phase 46 widens to those.
///
/// Note: we deliberately use [`TypeContext::add_tableexpr_param`] for
/// the joined alias because:
///   1. it registers both bare-column lookups and qualified
///      `alias.col` lookups, which the body checker needs;
///   2. it adds the alias to `tableexpr_param_schemas` so
///      Phase 17's [`infer_tableexpr_return_schema`] can expand
///      `alias.*` projections from the joined schema; and
///   3. it makes the joined columns visible to Phase 15's shadow
///      check (a parameter colliding with a joined column should
///      shadow-warn, just like a TableExpr-param column).
#[allow(clippy::type_complexity)]
pub fn register_join_alias_schemas(
    body_ctx: &mut TypeContext,
    select_stmt: &SelectStmt,
    sig: &FunctionSig,
    table_ref_schema_lookup: &dyn Fn(&TableRef) -> Option<Vec<(String, TypedColumn)>>,
) {
    let Some(from) = select_stmt.from_clause() else {
        return;
    };

    // Helper: is `ident` a TableExpr parameter on this signature?
    let is_tableexpr_param_name = |ident: &str| -> bool {
        sig.params
            .iter()
            .any(|p| p.name == ident && is_tableexpr_param(p))
    };

    let mut process_table_ref = |table_ref: &TableRef| {
        let alias = match table_ref.alias() {
            Some(a) => a,
            None => return,
        };

        // Bare identifier that matches a TableExpr parameter name —
        // register the alias as a synonym.
        if let Some(ident) = table_ref.identifier() {
            if is_tableexpr_param_name(&ident) {
                // Also seed the alias's column schema so qualified
                // refs `alias.col` resolve via the param's schema.
                let cols = body_ctx.tableexpr_param_columns(&ident).map(|s| s.to_vec());
                if let Some(cols) = cols {
                    body_ctx.add_tableexpr_param(&alias, &cols);
                } else {
                    body_ctx.add_alias(&alias, &ident);
                }
                return;
            }
        }

        // Otherwise, ask the closure to resolve a schema. Supported
        // shapes today: `smelt.models.X`, `smelt.sources.a.b`, and
        // `smelt.fn.<name>(...)` returning TableExpr. Unsupported
        // shapes (subqueries, CTE refs, derived tables) return None
        // and are deferred to Phase 46.
        if let Some(cols) = table_ref_schema_lookup(table_ref) {
            body_ctx.add_tableexpr_param(&alias, &cols);
        }
    };

    for table_ref in from.table_refs() {
        process_table_ref(&table_ref);
    }
    for join in from.joins() {
        if let Some(tr) = join.table_ref() {
            process_table_ref(&tr);
        }
    }
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
///   - nested `smelt.functions.*` path-form calls inside the SELECT-list
///     expressions dispatch through `nested_handler` so frames stack across
///     expansion depth (Phase 12 contract).
pub(crate) fn check_function_select_body(
    _sig: &FunctionSig,
    select_stmt: &SelectStmt,
    text: &str,
    body_ctx: &TypeContext,
    nested_handler: &NestedPathCallHandler<'_>,
    _nested_path_handler: Option<&NestedPathCallHandler<'_>>,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    // 1. Bare-column / qualified-access resolution. Any identifier the
    //    SELECT references that does not resolve in `body_ctx` emits
    //    `UnknownIdentifier`. Select-list aliases are handled by
    //    `check_undeclared_columns` already.
    //
    //    Phase C (meta-Text-as-identifier lift): `check_undeclared_columns`
    //    sees `<columnref-param>.<field>` (e.g. `c.name`) as an unresolved
    //    qualified column reference — it looks up `c.name` in model / source /
    //    CTE scopes, finds nothing, and would emit a spurious `UnknownIdentifier`
    //    before the lift check gets a chance to reinterpret the expression.
    //    To avoid the double-diagnostic, we suppress any
    //    `check_undeclared_columns` entry whose `qualifier` is the name of a
    //    `ColumnRef`-typed function parameter.  The lift check below then emits
    //    its own diagnostic (anchored at the correct span) if the lifted
    //    identifier does not resolve.
    for info in check_undeclared_columns(select_stmt, body_ctx) {
        // Suppress spurious UnknownIdentifier for `<columnref-binding>.<field>`
        // references regardless of whether `<field>` is a recognised ColumnRef
        // field.  Two sub-cases:
        //
        //   - Recognised field (e.g. `c.name`): handled by the meta-Text lift
        //     check in step 1b below.
        //   - Unrecognised field (e.g. `c.foo`): handled by
        //     `check_column_ref_field_diagnostics` in step 1c below, which
        //     emits `ColumnRefFieldUnknown` at the field-name token span (the
        //     correct spec-required anchor).
        //
        // Suppressing both avoids double-diagnostics: `check_undeclared_columns`
        // anchors at the whole `c.foo` expression, while the Phase C checks
        // anchor at the field token or emit the correct Phase C code.
        if let Some(qualifier) = &info.qualifier {
            use smelt_types::signatures::SmeltType;
            let qualifier_smelt_ty = body_ctx.lookup_function_param_smelt_type(qualifier);
            let qualifier_is_meta_record = qualifier_smelt_ty
                .map(|ty| {
                    matches!(
                        ty,
                        SmeltType::ColumnRef | SmeltType::ModelRef | SmeltType::SourceRef
                    )
                })
                .unwrap_or(false);
            if qualifier_is_meta_record {
                // All `<meta-record-param>.<field>` references are handled by
                // the lift check (step 1b) for recognised ColumnRef fields and by
                // `check_column_ref_field_diagnostics` (step 1c) /
                // `check_model_ref_source_ref_field_diagnostics` (step 1d) for
                // unrecognised fields. Skip here to avoid double-diagnostics.
                continue;
            }
        }
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

    // 1b. Meta-Text-as-identifier lift recognition (§"Meta-Text-as-identifier
    //     lift").  `check_meta_text_lift_diagnostics` detects expressions in
    //     SELECT / ORDER BY / GROUP BY that are meta-Text values
    //     (`<columnref-param>.name`) but does NOT emit scope diagnostics at
    //     body-check time.  The field-name token ("name") is not the
    //     per-element column name that the lift resolves to at expansion time;
    //     validating it against in-scope columns would produce false positives.
    //     Expansion-time validation is the correct location (spec §Semantics
    //     rule 6).  This loop is retained as a forward-compatibility hook for
    //     any future diagnostics the function may emit.
    for info in check_meta_text_lift_diagnostics(select_stmt, body_ctx) {
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

    // 1c. Phase C ColumnRef field validation (Phase C §"ColumnRef field access").
    //     For each `<columnref-param>.<field>` expression where `<field>` is NOT
    //     in the closed field set `{name, type, is_numeric}`, emit
    //     `ColumnRefFieldUnknown` anchored at the field-name token.
    //
    //     This is separate from step 1a (which now suppresses ALL ColumnRef-
    //     qualified references to avoid double-diagnostics) and step 1b (which
    //     handles the valid `c.name` lift). Only *unrecognised* fields reach
    //     this check.
    for diag in check_column_ref_field_diagnostics(select_stmt, body_ctx, text) {
        diagnostics.push(diag);
    }

    // 1d. Phase D ModelRef / SourceRef field validation.
    //     For each `<modelref-param>.<field>` / `<sourceref-param>.<field>`
    //     expression where `<field>` is NOT in the closed four-field set
    //     `{path, name, tags, columns}`, emit `ModelRefFieldUnknown` /
    //     `SourceRefFieldUnknown` anchored at the field-name token.
    //
    //     Also fires `check_wide_reflection_diagnostics` so that
    //     `smelt.models.with_tag(42)` inside a function body gets
    //     `WithTagRequiresText`, and `smelt.models.bogus()` gets
    //     `WideReflectionUnknownAccessor`.
    for diag in check_model_ref_source_ref_field_diagnostics(select_stmt, body_ctx, text) {
        diagnostics.push(diag);
    }
    for diag in check_wide_reflection_diagnostics(select_stmt, body_ctx, text) {
        diagnostics.push(diag);
    }

    // 2. Dispatch nested `smelt.functions.*` path-form calls so frames stack
    //    up across expansion depth. We walk every expression in the SELECT and
    //    hand any SMELT_PATH_CALL node to `nested_handler`.
    walk_select_columns_with_visitor(
        select_stmt,
        body_ctx,
        None,
        &mut |_qualifier, _name, _expr_type, _range| {
            // `walk_select_columns_with_visitor` hits leaf column refs,
            // not nested function calls. We walk SMELT_PATH_CALL nodes
            // separately below.
        },
    );

    // Walk every `SMELT_PATH_CALL` in the SELECT statement and let the
    // nested handler produce diagnostics with merged frame stacks.
    use smelt_parser::syntax_kind::SyntaxKind;
    for node in select_stmt.syntax().descendants() {
        if node.kind() == SyntaxKind::SMELT_PATH_CALL {
            if let Some(call) = SmeltPathCall::cast(node) {
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
                if !tr.is_function_call() && tr.smelt_path_call().is_none() {
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
///
/// When a CTE body has the shape `SELECT * FROM smelt.functions.<name>(<args>)`,
/// the CTE is marked opaque so outer bare-column references don't fire
/// spurious `UnknownIdentifier` diagnostics. Full schema resolution for
/// path-call CTE sources is deferred to a follow-on phase.
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

        // If the CTE source is a `smelt.functions.*` path call
        // (SMELT_PATH_CALL), mark the CTE opaque so outer bare-column
        // references don't fire spurious `UnknownIdentifier` diagnostics.
        // Full schema resolution for path-call CTE sources is deferred to a
        // follow-on phase.
        {
            let has_path_call_source = dfs
                .ctes
                .get(cte_name)
                .and_then(|c| c.query())
                .and_then(|q| q.select_stmt())
                .and_then(|s| {
                    let has_wildcard = s
                        .select_list()
                        .map(|sl| sl.items().any(|item| item.is_wildcard()))
                        .unwrap_or(false);
                    if !has_wildcard {
                        return None;
                    }
                    s.from_clause()
                        .and_then(|fc| fc.table_refs().find_map(|tr| tr.smelt_path_call()))
                })
                .is_some();

            // Bare `smelt.functions.*(...) PASSING ...` CTE body shape:
            // no SELECT_STMT child in the CTE, but a SMELT_PATH_CALL exists.
            let has_path_call_direct = {
                use smelt_parser::syntax_kind::SyntaxKind;
                dfs.ctes
                    .get(cte_name)
                    .map(|c| {
                        let cte_node = c.syntax();
                        let has_select = cte_node
                            .descendants()
                            .any(|d| d.kind() == SyntaxKind::SELECT_STMT);
                        let has_path = cte_node
                            .descendants()
                            .any(|d| d.kind() == SyntaxKind::SMELT_PATH_CALL);
                        !has_select && has_path
                    })
                    .unwrap_or(false)
            };

            if has_path_call_source || has_path_call_direct {
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
        if table_ref.is_function_call() || table_ref.smelt_path_call().is_some() {
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
                .filter(|tr| !tr.is_function_call() && tr.smelt_path_call().is_none())
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
/// Return `true` when `expr` is a bare unqualified identifier that names a
/// fragment-typed (`SelectItems<Kind>`) parameter registered in `ctx` (Phase 44b).
///
/// Used by [`check_fragment_context_bindings`] to skip column validation for
/// PASSING bodies that simply forward an outer function's fragment parameter
/// as-is (e.g. `PASSING metrics AS (metrics)`). There are no concrete column
/// references in such an expression — it's an opaque splice that inherits its
/// kind from the outer parameter declaration.
fn is_bare_fragment_param_ref(expr: &Expr, ctx: &TypeContext) -> bool {
    if let Some(col_ref) = expr.as_column_ref() {
        if col_ref.qualifier().is_none() {
            return ctx.is_fragment_param(col_ref.name());
        }
    }
    false
}

/// `body_ctx` must already be seeded with the call-site [`TableExpr`] schemas
/// (as `check_smelt_path_call` does before calling this function).
/// `caller_ctx` is the context at the call site — used for kind inference of
/// argument expressions, which live in the caller's scope (Phase 44b).
/// `text` is the call-site source text used to convert [`TextRange`]s to
/// [`Range`]s.
///
/// Pure — does not touch Salsa.
pub fn check_fragment_context_bindings(
    sig: &FunctionSig,
    select: &SelectStmt,
    body_ctx: &TypeContext,
    caller_ctx: &TypeContext,
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
        // Phase 44b: argument expressions live in the caller's scope, so kind
        // inference uses `caller_ctx` (not the callee's `body_ctx`). This
        // allows a bare reference to a fragment-typed parameter of the enclosing
        // function to inherit that parameter's declared kind rather than
        // defaulting to `Scalar`.
        //
        // If the arg is a bare reference to a fragment-typed parameter visible
        // in `body_ctx` (i.e. it forwards the callee's own fragment param as-is,
        // as in `PASSING metrics AS (metrics)`), skip the kind check entirely —
        // the forwarding is always valid by construction.
        if let Some(Ok(SmeltType::SelectItems { kind: req_kind, .. })) = &param.type_ref {
            if let Some((arg_expr, arg_range)) = bindings.get(&param.name) {
                // Phase 44b: skip kind check for bare fragment-param references
                // (these forward an opaque SelectItems value as-is).
                if !is_bare_fragment_param_ref(arg_expr, body_ctx) {
                    let found_kind = infer_expression_kind(arg_expr, caller_ctx);
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
            // Phase 44b: if the entire argument expression is a bare reference
            // to a fragment-typed parameter of the enclosing function, skip
            // column validation. The parameter forwards an opaque SelectItems
            // value as-is — there are no concrete column references to validate
            // against the splice context.
            if is_bare_fragment_param_ref(arg_expr, body_ctx) {
                continue;
            }
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

/// Expand `..spread_name` placeholders inside a brace-struct literal body
/// to explicit `spread_name.field AS field` references (Phase 37).
///
/// Given a body string like `{EXTRACT(HOUR FROM event.ts) AS hour, ..event}`
/// and extras `[(user_id, Integer)]`, produces:
/// `{EXTRACT(HOUR FROM event.ts) AS hour, event.user_id AS user_id}`.
///
/// - `body_text`: the raw body string (the entire brace-struct expression,
///   including the outer `{…}` braces).
/// - `spread_name`: the parameter name whose spread (`..spread_name`) to
///   replace (e.g. `"event"`).
/// - `extras`: the `(field_name, DataType)` list bound to the row variable;
///   each entry becomes `spread_name.field_name AS field_name`.
///
/// If `..spread_name` does not appear in `body_text`, returns `body_text`
/// unchanged.
///
/// Pure — no Salsa dependency.
pub fn expand_brace_struct_body(
    body_text: &str,
    spread_name: &str,
    extras: &[(String, DataType)],
) -> String {
    let spread_placeholder = format!("..{}", spread_name);

    if !body_text.contains(&spread_placeholder) {
        return body_text.to_string();
    }

    let expanded_fields: String = extras
        .iter()
        .map(|(field_name, _)| format!("{}.{} AS {}", spread_name, field_name, field_name))
        .collect::<Vec<_>>()
        .join(", ");

    // Replace `..spread_name` with the explicit field list.
    // Handle two cases: `..spread_name` preceded by `, ` or just `,`
    // (with or without a space), and standalone (no preceding comma, e.g.
    // when it's the first and only item — edge case but handle gracefully).
    let mut result = body_text.replace(&spread_placeholder, &expanded_fields);
    // Clean up `,,` or `, ,` that could arise if extras is empty.
    result = result.replace(", ,", ",").replace(",,", ",");
    // Clean up `,  ` before `}` if extras is empty.
    if extras.is_empty() {
        result = result.replace(", }", "}").replace(",}", "}");
    }
    result
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

// ─── Phase 42: smelt.as_struct() backend printer ──────────────────────────────
//
// The lowering helpers moved to `smelt-planner::lowering::as_struct` in
// Phase 42 — SQL emission is a planner concern. Re-exported here so the
// existing `smelt-db::function_body_check::{as_struct_to_sql,
// backend_supports_struct_literal}` call sites (and the published unit
// tests in `crates/smelt-db/tests/as_struct_tests.rs`) keep working.
pub use smelt_planner::lowering::{as_struct_to_sql, backend_supports_struct_literal};

/// Phase C (meta-language) HOF dispatcher: check a SELECT statement for
/// `ColumnRefFieldUnknown` errors that arise when a HOF lambda body accesses
/// an invalid field on a `ColumnRef`-typed binding.
///
/// A HOF call whose **first argument** is a `smelt.columns_of(…)` path call
/// produces a `List<ColumnRef>` source list.  Each lambda bound from that list
/// receives a `ColumnRef`-typed parameter.  When the lambda body accesses a
/// field that is not in the closed field set `{name, type, is_numeric}`, this
/// function emits `ColumnRefFieldUnknown` anchored at the field-name token.
///
/// Algorithm (per spec §ColumnRef field access):
/// 1. Walk every `FUNCTION_CALL` descendant of `select_stmt`.
/// 2. Identify `map`/`filter` calls whose first argument is
///    `smelt.columns_of(…)` (detected by AST shape: a `SMELT_PATH_CALL` node
///    whose single segment is `"columns_of"`).
/// 3. Extract the single-parameter lambda from the second argument.
/// 4. Build a scratch `TypeContext` with the lambda parameter registered as
///    `SmeltType::ColumnRef` (via `add_function_param_smelt_type`) so that
///    `check_column_ref_field_diagnostics_for_expr` can identify it.
/// 5. Walk the lambda body expression, emitting `ColumnRefFieldUnknown` for
///    every `<param>.<field>` access where `<field>` is not `name`, `type`,
///    or `is_numeric`.
///
/// Pure function — no Salsa dependency.
pub fn check_hof_column_ref_field_diagnostics(
    select_stmt: &SelectStmt,
    text: &str,
) -> Vec<Diagnostic> {
    use smelt_parser::SyntaxKind::{DOT, FUNCTION_CALL, IDENT, SMELT_PATH_CALL};

    let mut diags = Vec::new();
    let root = select_stmt.syntax();

    for node in root.descendants() {
        if node.kind() != FUNCTION_CALL {
            continue;
        }
        let call = match FunctionCall::cast(node.clone()) {
            Some(c) => c,
            None => continue,
        };

        // Get the HOF name, handling keyword-lexed names like `filter`.
        let call_name_lc: String = call
            .name()
            .unwrap_or_else(|| {
                call.syntax()
                    .children_with_tokens()
                    .filter_map(|e| e.into_token())
                    .find(|t| !t.kind().is_trivia())
                    .map(|t| t.text().to_string())
                    .unwrap_or_default()
            })
            .to_lowercase();

        // Only map/filter (reduce doesn't take a lambda).
        if call_name_lc != "map" && call_name_lc != "filter" {
            continue;
        }

        let args = call.arguments();
        if args.len() < 2 {
            continue;
        }

        // First arg: must be a smelt.columns_of(…) path call.
        let first_arg = &args[0];
        let is_columns_of = first_arg.syntax().descendants().any(|n| {
            if n.kind() != SMELT_PATH_CALL {
                return false;
            }
            let path_call = match SmeltPathCall::cast(n) {
                Some(p) => p,
                None => return false,
            };
            let segs = path_call.segments();
            segs.len() == 1 && segs[0].to_lowercase() == "columns_of"
        }) || {
            // Direct SMELT_PATH_CALL at the arg level.
            if first_arg.syntax().kind() == SMELT_PATH_CALL {
                if let Some(pc) = SmeltPathCall::cast(first_arg.syntax().clone()) {
                    let segs = pc.segments();
                    segs.len() == 1 && segs[0].to_lowercase() == "columns_of"
                } else {
                    false
                }
            } else {
                false
            }
        };

        if !is_columns_of {
            continue;
        }

        // Second arg: extract lambda parameter name and body.
        let second_arg = &args[1];
        let lambda: Option<Lambda> = Lambda::cast(second_arg.syntax().clone())
            .or_else(|| second_arg.syntax().children().find_map(Lambda::cast));
        let lambda = match lambda {
            Some(l) => l,
            None => continue,
        };

        let param_name = match lambda.params().into_iter().next() {
            Some(n) => n,
            None => continue,
        };
        let lambda_body = match lambda.body() {
            Some(b) => b,
            None => continue,
        };

        // Build a scratch context with the lambda param registered as ColumnRef.
        let mut lambda_ctx = TypeContext::new();
        lambda_ctx.add_function_param_smelt_type(&param_name, SmeltType::ColumnRef);
        // Also register as an Unknown-typed scalar so bare `param` resolves
        // via `lookup_identifier` (prevents spurious UnknownIdentifier).
        lambda_ctx.add_function_param(
            &param_name,
            TypedColumn {
                data_type: DataType::Unknown,
                nullable: true,
            },
        );

        // Walk the lambda body expression for ColumnRef field errors.
        let body_root = lambda_body.syntax();
        // Dedup tracker: same (qualifier, field) may appear in nested EXPRESSION
        // wrappers — emit only once per pair.
        let mut seen: std::collections::HashSet<(String, String)> =
            std::collections::HashSet::new();

        let to_range = |range: rowan::TextRange| -> Range {
            smelt_parser::ast::text_range_to_range(text, range)
        };

        for body_node in body_root.descendants() {
            let expr = match Expr::cast(body_node.clone()) {
                Some(e) => e,
                None => continue,
            };

            // Look for qualified column refs: `qualifier.field`.
            let col_ref = match smelt_parser::ast::ColumnRef::from_expr(&expr) {
                Some(cr) => cr,
                None => continue,
            };

            let qualifier = match col_ref.qualifier() {
                Some(q) => q,
                None => continue,
            };

            // Is the qualifier registered as ColumnRef?
            let is_column_ref = lambda_ctx
                .lookup_function_param_smelt_type(qualifier)
                .map(|ty| matches!(ty, SmeltType::ColumnRef))
                .unwrap_or(false);

            if !is_column_ref {
                continue;
            }

            let field_name = col_ref.name();

            // Valid field — skip.
            if column_ref_field(field_name).is_some() {
                continue;
            }

            // Deduplicate.
            let key = (qualifier.to_string(), field_name.to_string());
            if !seen.insert(key) {
                continue;
            }

            // Find the field-name token (the IDENT after the DOT) for precise anchoring.
            let field_token_range = {
                let mut after_dot = false;
                let mut found: Option<rowan::TextRange> = None;
                for token in body_node
                    .descendants_with_tokens()
                    .filter_map(|e| e.into_token())
                {
                    let kind = token.kind();
                    if kind == DOT {
                        after_dot = true;
                    } else if kind == IDENT && after_dot {
                        found = Some(token.text_range());
                        break;
                    }
                }
                found.unwrap_or_else(|| body_node.text_range())
            };

            diags.push(Diagnostic {
                severity: DiagnosticSeverity::Error,
                message: crate::meta_reflection_diagnostic_message(
                    DiagnosticCode::ColumnRefFieldUnknown,
                    None,
                    Some(field_name),
                ),
                range: to_range(field_token_range),
                code: Some(DiagnosticCode::ColumnRefFieldUnknown),
                data: None,
            });
        }
    }

    diags
}

/// Phase D (meta-language) — HOF lambda body check for `ModelRef` / `SourceRef`
/// field projections.
///
/// Mirrors `check_hof_column_ref_field_diagnostics` for the `ModelRef` /
/// `SourceRef` record types.  For each `map` / `filter` HOF call whose first
/// argument is a wide-reflection accessor call (`smelt.models.*` or
/// `smelt.sources.*`), this function:
///
/// 1. Determines the element type of the list (`ModelRef` for `smelt.models.*`,
///    `SourceRef` for `smelt.sources.*`).
/// 2. Extracts the lambda parameter name.
/// 3. Builds a scratch `TypeContext` with the parameter registered as the
///    appropriate meta-record type.
/// 4. Walks the lambda body for `<param>.<field>` accesses where `<field>` is
///    not in the closed field set `{path, name, tags, columns}` and emits
///    `ModelRefFieldUnknown` / `SourceRefFieldUnknown` anchored at the
///    field-name token.
///
/// Pure function — no Salsa dependency.
pub fn check_hof_model_ref_source_ref_field_diagnostics(
    select_stmt: &SelectStmt,
    text: &str,
) -> Vec<Diagnostic> {
    use smelt_parser::SyntaxKind::{DOT, FUNCTION_CALL, IDENT, SMELT_PATH_CALL};
    use smelt_types::signatures::{model_ref_field, source_ref_field, SmeltType};

    let mut diags = Vec::new();
    let root = select_stmt.syntax();

    for node in root.descendants() {
        if node.kind() != FUNCTION_CALL {
            continue;
        }
        let call = match FunctionCall::cast(node.clone()) {
            Some(c) => c,
            None => continue,
        };

        // Only map/filter HOFs.
        let call_name_lc: String = call
            .name()
            .unwrap_or_else(|| {
                call.syntax()
                    .children_with_tokens()
                    .filter_map(|e| e.into_token())
                    .find(|t| !t.kind().is_trivia())
                    .map(|t| t.text().to_string())
                    .unwrap_or_default()
            })
            .to_lowercase();
        if call_name_lc != "map" && call_name_lc != "filter" {
            continue;
        }

        let args = call.arguments();
        if args.len() < 2 {
            continue;
        }

        // First arg: must be a wide-reflection smelt path call
        // (`smelt.models.*` or `smelt.sources.*`).
        let first_arg = &args[0];
        let wide_reflection_ns: Option<SmeltType> = first_arg
            .syntax()
            .descendants()
            .find_map(|n| {
                if n.kind() != SMELT_PATH_CALL {
                    return None;
                }
                let path_call = SmeltPathCall::cast(n)?;
                let segs = path_call.segments();
                if segs.len() != 2 {
                    return None;
                }
                match segs[0].to_lowercase().as_str() {
                    "models" => Some(SmeltType::ModelRef),
                    "sources" => Some(SmeltType::SourceRef),
                    _ => None,
                }
            })
            .or_else(|| {
                if first_arg.syntax().kind() != SMELT_PATH_CALL {
                    return None;
                }
                let pc = SmeltPathCall::cast(first_arg.syntax().clone())?;
                let segs = pc.segments();
                if segs.len() != 2 {
                    return None;
                }
                match segs[0].to_lowercase().as_str() {
                    "models" => Some(SmeltType::ModelRef),
                    "sources" => Some(SmeltType::SourceRef),
                    _ => None,
                }
            });

        let elem_ty = match wide_reflection_ns {
            Some(ty) => ty,
            None => continue,
        };

        // Second arg: extract lambda parameter name and body.
        let second_arg = &args[1];
        let lambda: Option<Lambda> = Lambda::cast(second_arg.syntax().clone())
            .or_else(|| second_arg.syntax().children().find_map(Lambda::cast));
        let lambda = match lambda {
            Some(l) => l,
            None => continue,
        };

        let param_name = match lambda.params().into_iter().next() {
            Some(n) => n,
            None => continue,
        };
        let lambda_body = match lambda.body() {
            Some(b) => b,
            None => continue,
        };

        // Build a scratch context with the lambda param registered as ModelRef or SourceRef.
        let mut lambda_ctx = TypeContext::new();
        lambda_ctx.add_function_param_smelt_type(&param_name, elem_ty.clone());
        // Also register as Unknown-typed scalar so bare `param` resolves
        // via `lookup_identifier` (prevents spurious UnknownIdentifier).
        use smelt_types::TypedColumn;
        lambda_ctx.add_function_param(
            &param_name,
            TypedColumn {
                data_type: smelt_types::DataType::Unknown,
                nullable: true,
            },
        );

        let is_model_ref = matches!(elem_ty, SmeltType::ModelRef);

        // Walk the lambda body for field errors.
        let body_root = lambda_body.syntax();
        let mut seen: std::collections::HashSet<(String, String)> =
            std::collections::HashSet::new();

        let to_range = |range: rowan::TextRange| -> Range {
            smelt_parser::ast::text_range_to_range(text, range)
        };

        for body_node in body_root.descendants() {
            let expr = match smelt_parser::ast::Expr::cast(body_node.clone()) {
                Some(e) => e,
                None => continue,
            };

            let col_ref = match smelt_parser::ast::ColumnRef::from_expr(&expr) {
                Some(cr) => cr,
                None => continue,
            };

            let qualifier = match col_ref.qualifier() {
                Some(q) => q,
                None => continue,
            };

            // Is the qualifier registered as ModelRef/SourceRef?
            let registered_ty = lambda_ctx
                .lookup_function_param_smelt_type(qualifier)
                .cloned();
            let matches_elem = match &registered_ty {
                Some(SmeltType::ModelRef) => is_model_ref,
                Some(SmeltType::SourceRef) => !is_model_ref,
                _ => false,
            };
            if !matches_elem {
                continue;
            }

            let field_name = col_ref.name();

            // Valid field — skip.
            let field_known = if is_model_ref {
                model_ref_field(field_name).is_some()
            } else {
                source_ref_field(field_name).is_some()
            };
            if field_known {
                continue;
            }

            // Deduplicate.
            let key = (qualifier.to_string(), field_name.to_string());
            if !seen.insert(key) {
                continue;
            }

            // Find the field-name token (the IDENT after the DOT).
            let field_token_range = {
                let mut after_dot = false;
                let mut found: Option<rowan::TextRange> = None;
                for token in body_node
                    .descendants_with_tokens()
                    .filter_map(|e| e.into_token())
                {
                    let kind = token.kind();
                    if kind == DOT {
                        after_dot = true;
                    } else if kind == IDENT && after_dot {
                        found = Some(token.text_range());
                        break;
                    }
                }
                found.unwrap_or_else(|| body_node.text_range())
            };

            let code = if is_model_ref {
                DiagnosticCode::ModelRefFieldUnknown
            } else {
                DiagnosticCode::SourceRefFieldUnknown
            };
            diags.push(Diagnostic {
                severity: DiagnosticSeverity::Error,
                message: crate::meta_reflection_diagnostic_message(code, None, Some(field_name)),
                range: to_range(field_token_range),
                code: Some(code),
                data: None,
            });
        }
    }

    diags
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

    // === Phase B (meta-language Phase 3) TDD tests: HOF expansion frames ===

    /// A type error inside a HOF lambda body carries an `ExpansionFrames` payload
    /// whose innermost frame has `function = "<map>"`, `fn_id = None`, and
    /// `call_site_range` set to the span of the `map(...)` call.
    ///
    /// This exercises the anonymous-frame stamping: HOF calls push a frame with
    /// `fn_id = None` (anonymous) and angle-bracketed name (`"<map>"`) before
    /// walking the lambda body diagnostics.
    #[test]
    fn hof_lambda_body_diagnostic_carries_anonymous_frame() {
        use crate::type_inference::{check_hof_position_diagnostics, TypeContext};

        // Parse a SELECT containing a map HOF whose lambda body has a type error:
        // `fn c => c + 'hello'` — `c` is SmallInt (from [1,2,3]), `'hello'` is Text.
        let sql = "SELECT map([1, 2, 3], fn c => c + 'hello') FROM t";
        let parse = smelt_parser::parse(sql);
        let ast = smelt_parser::ast::File::cast(parse.syntax()).expect("file");
        let select = ast.select_stmt().expect("one SELECT");
        let ctx = TypeContext::new();
        let diags = check_hof_position_diagnostics(&select, &ctx, sql);

        // There must be at least one diagnostic stamped with an anonymous HOF frame.
        // The function field must use the angle-bracketed form `"<map>"` per spec §FrameInfo.
        let frame_diag = diags.iter().find(|d| {
            matches!(
                &d.data,
                Some(crate::DiagnosticData::ExpansionFrames(frames))
                    if frames.iter().any(|f| f.fn_id.is_none() && f.function == "<map>")
            )
        });
        assert!(
            frame_diag.is_some(),
            "HOF lambda body diagnostic must carry an anonymous <map> frame (angle-bracketed); got: {diags:?}"
        );
    }

    /// `walk_hof_lambda_body_with_anonymous_frame` with `nested = Some(handler)`
    /// propagates the handler into `walk_body` so SMELT_PATH_CALL nodes inside
    /// the lambda body still receive frame stamping.
    ///
    /// Also verifies the `<map>` bracket form: even when `nested` is wired,
    /// the anonymous HOF frame carries `function = "<map>"`.
    #[test]
    fn hof_lambda_body_with_nested_wired_propagates_and_brackets_hof_name() {
        use crate::type_inference::TypeContext;

        // Parse a smelt.define whose body has an unknown identifier `bad_expr`.
        // `walk_body` will emit `UnknownIdentifier`; the anonymous HOF frame wraps it.
        let (_sig, body_expr, text) =
            parse_define("smelt.define f(x: Expr<Integer>) AS (bad_expr)\n");

        // Lambda context has no bindings — `bad_expr` is unknown.
        let lambda_ctx = TypeContext::new();

        // A nested handler that simply returns no diagnostics (no SMELT_PATH_CALL
        // nodes exist in this body anyway — we verify `nested` is wired without
        // panicking, and that the HOF frame is still stamped correctly).
        let nested_fn: &NestedPathCallHandler<'_> = &|_call, _ctx, _text| vec![];

        let diags = walk_hof_lambda_body_with_anonymous_frame(
            &body_expr,
            &lambda_ctx,
            &text,
            "map",
            None,
            Some(nested_fn),
        );

        // The unknown identifier should produce an UnknownIdentifier diagnostic
        // wrapped with a `<map>` anonymous HOF frame (angle-bracketed form).
        let frame_diag = diags.iter().find(|d| {
            matches!(
                &d.data,
                Some(DiagnosticData::ExpansionFrames(frames))
                    if frames.iter().any(|f| f.fn_id.is_none() && f.function == "<map>")
            )
        });
        assert!(
            frame_diag.is_some(),
            "walk_hof_lambda_body_with_anonymous_frame with nested=Some must stamp \
             a <map> anonymous frame (angle-bracketed); got: {diags:?}"
        );
    }

    // ─── Phase C Phase 2: pipeline integration tests ──────────────────────────
    //
    // These tests verify that `check_function_select_body` (called from the real
    // diagnostic pipeline) correctly handles `<columnref-param>.<field>` references:
    //   - No spurious `UnknownIdentifier` emitted by `check_undeclared_columns`.
    //   - Lift diagnostics from `check_meta_text_lift_diagnostics` are wired in.
    //
    // We call `check_function_select_body` directly (it is private but we are in
    // the same module) rather than going through the full Salsa pipeline, which
    // would require a real database and file system.
    //
    // `check_function_select_body` accepts `_sig: &FunctionSig` but does NOT
    // inspect it internally (the leading underscore is intentional); we parse a
    // minimal smelt.define and use the extracted sig purely to satisfy the type.

    /// Parse a minimal `smelt.define` to obtain a `FunctionSig` for passing to
    /// `check_function_select_body`.  The sig is not inspected by the function.
    fn minimal_sig() -> FunctionSig {
        let text = "smelt.define test_fn(x: Expr<Integer>) AS (x)\n";
        let clean = smelt_parser::strip_frontmatter(text).to_string();
        let p = smelt_parser::parse(&clean);
        let ast = AstFile::cast(p.syntax()).expect("FILE");
        extract_function_signatures(&ast, &clean)
            .into_iter()
            .next()
            .expect("one define")
    }

    /// Build a `TypeContext` seeded with:
    ///   - `c: ColumnRef` (function parameter via `add_function_param_smelt_type`)
    ///   - `c` also as an `Unknown`-typed `function_param` so bare `c` resolves
    ///   - model columns `t.name (Text)` and `t.amount (Double)`
    ///   - alias `t → t`
    fn make_pipeline_ctx() -> TypeContext {
        use smelt_types::signatures::SmeltType;
        let mut ctx = TypeContext::new();
        // Register the ColumnRef smelt-type so is_meta_text_value fires and
        // the suppression check in check_function_select_body triggers.
        ctx.add_function_param_smelt_type("c", SmeltType::ColumnRef);
        // Also register `c` as an Unknown-typed function param so bare `c`
        // resolves via lookup_identifier (prevents a spurious UnknownIdentifier
        // for the bare qualifier itself).
        ctx.add_function_param(
            "c",
            smelt_types::TypedColumn {
                data_type: smelt_types::DataType::Unknown,
                nullable: true,
            },
        );
        ctx.add_model_column(
            "t",
            "name",
            smelt_types::TypedColumn {
                data_type: smelt_types::DataType::Text,
                nullable: true,
            },
        );
        ctx.add_model_column(
            "t",
            "amount",
            smelt_types::TypedColumn {
                data_type: smelt_types::DataType::Double,
                nullable: true,
            },
        );
        ctx.add_alias("t", "t");
        ctx
    }

    /// Parse a SELECT statement from a bare SQL string (no smelt.define wrapper).
    fn parse_select_for_pipeline(sql: &str) -> smelt_parser::ast::SelectStmt {
        let parse = smelt_parser::parse(sql);
        let ast = smelt_parser::ast::File::cast(parse.syntax()).expect("FILE");
        ast.select_stmt().expect("one SELECT")
    }

    /// `check_function_select_body` with `c: ColumnRef` and body
    /// `SELECT c.name FROM t` where `name` is in scope must emit ZERO diagnostics —
    /// no spurious `UnknownIdentifier` from `check_undeclared_columns` and no
    /// lift error from `check_meta_text_lift_diagnostics`.
    #[test]
    fn pipeline_column_ref_name_in_scope_produces_no_diagnostics() {
        let sig = minimal_sig();
        let ctx = make_pipeline_ctx();
        let sql = "SELECT c.name FROM t";
        let select = parse_select_for_pipeline(sql);

        let no_op_nested: &NestedPathCallHandler<'_> = &|_call, _ctx, _text| vec![];

        let diags = check_function_select_body(&sig, &select, sql, &ctx, no_op_nested, None);

        assert!(
            diags.is_empty(),
            "SELECT c.name FROM t with c: ColumnRef and 'name' in scope must produce \
             ZERO diagnostics from the pipeline — no spurious UnknownIdentifier and no \
             lift error; got: {diags:?}"
        );
    }

    /// `check_function_select_body` with `c: ColumnRef` and body
    /// `SELECT c.foo FROM t` where `foo` is NOT a recognised ColumnRef field must
    /// emit EXACTLY ONE `ColumnRefFieldUnknown` (Phase C spec).
    ///
    /// The suppression gate fires for ALL ColumnRef-qualified references
    /// (`qualifier_is_column_ref` is sufficient) to avoid double-diagnostics.
    /// `check_column_ref_field_diagnostics` (step 1c) then emits `ColumnRefFieldUnknown`
    /// anchored at the field-name token, which is the Phase C spec-required behaviour.
    #[test]
    fn pipeline_column_ref_non_text_field_emits_column_ref_field_unknown() {
        let sig = minimal_sig();
        let ctx = make_pipeline_ctx();
        let sql = "SELECT c.foo FROM t";
        let select = parse_select_for_pipeline(sql);

        let no_op_nested: &NestedPathCallHandler<'_> = &|_call, _ctx, _text| vec![];

        let diags = check_function_select_body(&sig, &select, sql, &ctx, no_op_nested, None);

        // `c.foo` — `foo` is not a recognised ColumnRef field (`column_ref_field("foo")` = None).
        // Phase C: the suppression gate fires for ALL ColumnRef-qualified refs, and
        // `check_column_ref_field_diagnostics` emits `ColumnRefFieldUnknown` for the unknown field.
        // This is one and only one diagnostic — no double-diagnostic from check_undeclared_columns.
        assert_eq!(
            diags.len(),
            1,
            "SELECT c.foo FROM t must produce exactly one ColumnRefFieldUnknown — \
             foo is not a recognised ColumnRef field; got: {diags:?}"
        );
        assert_eq!(
            diags[0].code,
            Some(DiagnosticCode::ColumnRefFieldUnknown),
            "diagnostic must be ColumnRefFieldUnknown; got: {:?}",
            diags[0].code
        );
    }

    /// `check_function_select_body` with `c: ColumnRef` and body
    /// `SELECT c.name FROM t` where no column literally named `"name"` exists in
    /// the body context must emit ZERO diagnostics.
    ///
    /// Body-check-time scope validation for lifted identifiers is suppressed:
    /// `check_meta_text_lift_diagnostics` returns the field-name token `"name"`,
    /// not the per-element column name that `c.name` resolves to at expansion time.
    /// Validating that token against in-scope columns produces false positives in
    /// the common case where no column happens to be literally named `"name"`.
    /// Expansion-time validation (after the per-element column name is known)
    /// is the correct location per spec §Semantics rule 6.
    #[test]
    fn pipeline_column_ref_name_not_in_scope_emits_no_diagnostics() {
        use smelt_types::signatures::SmeltType;

        let sig = minimal_sig();

        // Build a context where `c` is ColumnRef but the model has NO `name` column.
        let mut ctx = TypeContext::new();
        ctx.add_function_param_smelt_type("c", SmeltType::ColumnRef);
        ctx.add_function_param(
            "c",
            smelt_types::TypedColumn {
                data_type: smelt_types::DataType::Unknown,
                nullable: true,
            },
        );
        // Only `amount` — no `name`.
        ctx.add_model_column(
            "t",
            "amount",
            smelt_types::TypedColumn {
                data_type: smelt_types::DataType::Double,
                nullable: true,
            },
        );
        ctx.add_alias("t", "t");

        let sql = "SELECT c.name FROM t";
        let select = parse_select_for_pipeline(sql);

        let no_op_nested: &NestedPathCallHandler<'_> = &|_call, _ctx, _text| vec![];

        let diags = check_function_select_body(&sig, &select, sql, &ctx, no_op_nested, None);

        // Zero diagnostics: body-check-time lift-scope validation is suppressed.
        // `check_undeclared_columns` is suppressed for the ColumnRef qualifier `c`
        // (step 1a), and `check_meta_text_lift_diagnostics` (step 1b) returns
        // no diagnostics because expansion-time column names are not yet known.
        assert!(
            diags.is_empty(),
            "SELECT c.name FROM t with 'name' NOT literally in scope must produce ZERO \
             diagnostics — body-check-time lift-scope validation is suppressed; got: {diags:?}"
        );
    }

    // ─── Phase C Phase 3 TDD tests ────────────────────────────────────────────

    /// `walk_hof_lambda_body_with_anonymous_frame` with a concrete
    /// `column_origin` stamps that origin onto the resulting frame.
    ///
    /// When the HOF source list comes from `smelt.columns_of(t)`, the
    /// per-element frame's `column_origin` must be set to the column's source
    /// span from the upstream schema. This test exercises the frame-stamping
    /// path by calling a variant that accepts an explicit `column_origin`.
    ///
    /// DEFERRED (Phase 5): The full integration path — where `smelt.columns_of(orders)`
    /// feeds a HOF source list and each per-element expansion frame carries
    /// `column_origin = Some(span_of_column_in_source_schema)` flowing from the
    /// `ModelSchema`'s column declaration spans — is deferred to Phase 5 (HOF
    /// expansion wiring). The current test calls
    /// `walk_hof_lambda_body_with_anonymous_frame_and_origin` directly with a
    /// hand-constructed span, which verifies the frame-stamping mechanism in
    /// isolation but does NOT exercise the `columns_of` source-list path. A full
    /// integration test requires the HOF expansion dispatcher to call
    /// `columns_of_for_table_expr`, iterate the resulting `ColumnRefValue`s, and
    /// pass each `ColumnRefValue::source_span` as the `column_origin` argument —
    /// that dispatcher wiring does not yet exist. Re-enable and rewrite once
    /// Phase 5 HOF expansion is complete.
    #[test]
    #[ignore = "deferred to Phase 5: HOF expansion wiring for smelt.columns_of source lists not yet integrated"]
    fn columns_of_hof_lambda_carries_column_origin_frame() {
        use crate::type_inference::TypeContext;
        use rowan::TextRange;
        use rowan::TextSize;

        // Set up a body expression that will produce an error (unknown identifier).
        let (_sig, body_expr, text) =
            parse_define("smelt.define f(x: Expr<Integer>) AS (bad_expr)\n");

        let lambda_ctx = TypeContext::new();

        // Simulate a concrete column origin span (e.g. column "id" declared at offset 7–9).
        let origin = TextRange::new(TextSize::from(7u32), TextSize::from(9u32));

        let diags = walk_hof_lambda_body_with_anonymous_frame_and_origin(
            &body_expr,
            &lambda_ctx,
            &text,
            "map",
            None,
            None,
            Some(origin),
        );

        // The frame must carry the column_origin we passed in.
        let frame_diag = diags.iter().find(|d| {
            matches!(
                &d.data,
                Some(DiagnosticData::ExpansionFrames(frames))
                    if frames.iter().any(|f| f.column_origin == Some(origin))
            )
        });
        assert!(
            frame_diag.is_some(),
            "HOF frame must carry the column_origin span; got: {diags:?}"
        );
    }

    /// `check_columns_of_unresolvable_schema` emits `ColumnsOfUnresolvableSchema`
    /// anchored at the call site and the surrounding splice drops (empty diagnostics
    /// from the HOF handler).
    ///
    /// The drop-on-error policy matches `MetaSpreadInForbiddenPosition`: the
    /// diagnostic is emitted and no further cascading errors surface from inside
    /// the lambda body.
    ///
    /// DEFERRED (Phase 5): The spec's drop-on-error invariant requires the HOF
    /// expansion dispatcher to: (a) call `columns_of_for_table_expr` which returns
    /// `Err(())` for an unresolvable model, (b) emit exactly one
    /// `ColumnsOfUnresolvableSchema` diagnostic anchored at the `smelt.columns_of`
    /// call-site span, and (c) suppress all cascading diagnostics from the lambda
    /// body for that HOF call.  That HOF expansion dispatcher wiring does not yet
    /// exist (Phase 5 scope).  The current test only exercises the message-function
    /// and the diagnostic code round-trip, not the pipeline invariant.  Re-enable
    /// and rewrite to set up a full workspace with a function containing
    /// `map(smelt.columns_of(t), fn c => SOME_BODY)` where `t` resolves to an
    /// `Unknown` schema, run `file_diagnostics`, and assert exactly one
    /// `ColumnsOfUnresolvableSchema` diagnostic with no cascading diagnostics from
    /// inside the lambda body.
    #[test]
    #[ignore = "deferred to Phase 5: ColumnsOfUnresolvableSchema drop-on-error HOF wiring not yet integrated"]
    fn columns_of_unresolvable_schema_drops_with_diagnostic() {
        // Build a diagnostic directly via the Phase C message function.
        let msg = crate::meta_reflection_diagnostic_message_with_table_expr(
            crate::DiagnosticCode::ColumnsOfUnresolvableSchema,
            None,
            None,
            Some("smelt.models.nonexistent"),
        );
        assert_eq!(
            msg,
            "cannot resolve column list for smelt.models.nonexistent; upstream schema is unknown",
            "ColumnsOfUnresolvableSchema message must match spec"
        );

        // Simulate constructing a drop diagnostic — the message is produced and
        // the surrounding splice would emit nothing else.
        let drop_diag = crate::Diagnostic {
            severity: crate::DiagnosticSeverity::Error,
            message: msg.clone(),
            range: crate::Range {
                start: smelt_parser::ast::Position { line: 0, column: 0 },
                end: smelt_parser::ast::Position { line: 0, column: 0 },
            },
            code: Some(crate::DiagnosticCode::ColumnsOfUnresolvableSchema),
            data: None,
        };
        // Verify the diagnostic round-trips correctly.
        assert_eq!(
            drop_diag.code,
            Some(crate::DiagnosticCode::ColumnsOfUnresolvableSchema)
        );
        assert!(drop_diag.message.contains("nonexistent"));
    }

    /// `column_origin_passed_through_to_first_frame` verifies that the
    /// `column_origin` argument supplied to
    /// `walk_hof_lambda_body_with_anonymous_frame_and_origin` is carried verbatim
    /// onto the resulting anonymous frame (single-element check).
    ///
    /// End-to-end ordering across multiple `ColumnRef` source elements is verified
    /// by `columns_of_expansion_preserves_source_ordering_pure` in `tests.rs`,
    /// which exercises `columns_to_column_ref_values` directly. Multi-element
    /// dispatcher integration is deferred (see plan's "Deferred during
    /// implementation" §"Phase 3 — production HOF expansion dispatcher
    /// integration").
    #[test]
    fn column_origin_passed_through_to_first_frame() {
        use crate::type_inference::TypeContext;
        use rowan::{TextRange, TextSize};

        let (_sig, body_expr, text) =
            parse_define("smelt.define f(x: Expr<Integer>) AS (bad_expr)\n");

        let lambda_ctx = TypeContext::new();

        // First column origin (column "id" at offset 0–2).
        let first_origin = TextRange::new(TextSize::from(0u32), TextSize::from(2u32));

        // Walk the lambda body with the first column's origin.
        let diags = walk_hof_lambda_body_with_anonymous_frame_and_origin(
            &body_expr,
            &lambda_ctx,
            &text,
            "map",
            None,
            None,
            Some(first_origin),
        );

        // The frame must have column_origin = Some(first_origin).
        let has_first_origin = diags.iter().any(|d| {
            matches!(
                &d.data,
                Some(DiagnosticData::ExpansionFrames(frames))
                    if frames.iter().any(|f| f.column_origin == Some(first_origin))
            )
        });
        assert!(
            has_first_origin,
            "frame for first column must carry first_origin; got: {diags:?}"
        );
    }

    /// `columns_of_for_table_expr` with two different models at call-site produces
    /// different concrete column lists, proving per-call-site schema-resolution works.
    ///
    /// This verifies the call-site materialisation behaviour: when a function
    /// `f(t: TableExpr)` is called with `orders` vs `products`, the resolved
    /// `ColumnRefValue` lists differ.  The test exercises the Salsa query
    /// `columns_of_for_table_expr` directly (the machinery that wires expansion-time
    /// call-site arguments to concrete schemas), without requiring the full HOF
    /// expansion pipeline which is deferred to Phase 5.
    #[test]
    fn columns_of_in_table_expr_parameter_uses_call_site_schema() {
        use crate::test_harness::TestDb;
        use std::path::PathBuf;
        use std::sync::Arc;

        let mut db = TestDb::default();

        // Model `orders` with columns: id, amount.
        let orders_path = PathBuf::from("models/orders.sql");
        db.set_file_text(
            orders_path.clone(),
            Arc::new("SELECT 1 AS id, 9.99 AS amount FROM source.raw_orders".to_string()),
        );

        // Model `products` with columns: sku, price.
        let products_path = PathBuf::from("models/products.sql");
        db.set_file_text(
            products_path.clone(),
            Arc::new("SELECT 'A1' AS sku, 4.99 AS price FROM source.raw_products".to_string()),
        );

        db.set_all_files(Arc::new(vec![orders_path.clone(), products_path.clone()]));
        db.set_file_project_root(orders_path.clone(), PathBuf::from("."));
        db.set_file_project_root(products_path.clone(), PathBuf::from("."));
        db.set_project_sources_yaml(PathBuf::from("."), Arc::new(String::new()));
        db.set_all_project_roots(Arc::new(vec![PathBuf::from(".")]));

        let ws = db.sync_workspace();

        // Call-site: f(orders) → columns [id, amount].
        let orders_cols = crate::columns_of_for_table_expr(&db.db, ws, "orders".to_string())
            .expect("columns_of_for_table_expr must resolve orders");
        assert_eq!(
            orders_cols.len(),
            2,
            "orders has 2 columns; got: {:?}",
            orders_cols
        );
        assert_eq!(orders_cols[0].name, "id");
        assert_eq!(orders_cols[1].name, "amount");

        // Call-site: f(products) → columns [sku, price].
        let products_cols = crate::columns_of_for_table_expr(&db.db, ws, "products".to_string())
            .expect("columns_of_for_table_expr must resolve products");
        assert_eq!(
            products_cols.len(),
            2,
            "products has 2 columns; got: {:?}",
            products_cols
        );
        assert_eq!(products_cols[0].name, "sku");
        assert_eq!(products_cols[1].name, "price");

        // The two call sites with different `t` arguments produce different column lists.
        let orders_names: Vec<&str> = orders_cols.iter().map(|c| c.name.as_str()).collect();
        let products_names: Vec<&str> = products_cols.iter().map(|c| c.name.as_str()).collect();
        assert_ne!(
            orders_names, products_names,
            "orders and products must produce different column lists; \
             per-call-site schema-resolution must not conflate models"
        );
    }
}
