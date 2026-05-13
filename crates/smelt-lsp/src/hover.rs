//! Hover and goto-definition text formatters.
//!
//! All `hover_text_for_*` builders, expansion-frame rendering, and related
//! pure formatting helpers. These functions are stateless: they take an AST
//! node (+ optional context) and return a `String`/`Option<PathBuf>`.
//!
//! Some of the `pub fn`s below are reachable only from the unit-test module
//! (`#[cfg(test)] mod tests;`) — they were exercised in-place when this file
//! was part of `lib.rs`. They are kept `pub` because they form a stable
//! formatter surface that the `Backend::hover`/`goto_definition` impls grow
//! into call sites for over time.
#![allow(dead_code)]

use tower_lsp::lsp_types::*;

use smelt_db::{Diagnostic as DbDiagnostic, DiagnosticData as DbData};
use smelt_types::format_smelt_type_hover;

/// Render a database diagnostic into the LSP message body + per-frame
/// `related_information` list.
///
/// Pulled out of [`Backend::to_lsp_diagnostic`] as a pure function so it
/// can be unit-tested without needing to construct a live `Backend`/LSP
/// `Client`. The returned tuple is `(message, related_information)`:
///
/// * `message` — the primary diagnostic text. If the diagnostic carries
///   an `ExpansionFrames` payload, one trailer line per frame is appended
///   in outer-to-inner order (Phase 12 §16 #16 Step 2 renderer).
/// * `related_information` — `Some(..)` with one entry per frame that
///   carries both `decl_path` and `decl_range`; `None` otherwise (no
///   frames, or no frame had resolvable location metadata).
///
/// The underlying `FrameInfo` vector is stored innermost-first →
/// outermost-last (the canonical merge order used by
/// `smelt_db::check_smelt_fn_call`), so we iterate in reverse to render
/// the outermost (user-visible) call first, then walk down to the
/// innermost cause.
pub fn render_expansion_frames(
    diag: &DbDiagnostic,
) -> (String, Option<Vec<DiagnosticRelatedInformation>>) {
    let mut message = diag.message.clone();
    let Some(DbData::ExpansionFrames(frames)) = diag.data.as_ref() else {
        return (message, None);
    };
    let mut related: Vec<DiagnosticRelatedInformation> = Vec::new();
    for frame in frames.iter().rev() {
        // Anonymous HOF frames (fn_id is None) have an empty `param` and
        // `bound_type`; rendering them with the named-frame template produces
        // awkward text like `"`, `` was bound to "`.  Use a shorter form that
        // names only the HOF (matching the spec for anonymous expansion frames).
        let is_anonymous = frame.fn_id.is_none();
        let trailer = if is_anonymous {
            format!("\nin expansion of `{}` call", frame.function)
        } else {
            format!(
                "\nin expansion of `{}`, `{}` was bound to {}",
                frame.function, frame.param, frame.bound_type,
            )
        };
        message.push_str(&trailer);
        if let (Some(path), Some(range)) = (&frame.decl_path, frame.decl_range.as_ref()) {
            if let Ok(uri) = Url::from_file_path(path) {
                let related_msg = if is_anonymous {
                    format!("in expansion of `{}` call", frame.function)
                } else {
                    format!(
                        "in expansion of `{}`, `{}` was bound to {}",
                        frame.function, frame.param, frame.bound_type,
                    )
                };
                related.push(DiagnosticRelatedInformation {
                    location: Location {
                        uri,
                        range: Range {
                            start: Position {
                                line: range.start.line,
                                character: range.start.column,
                            },
                            end: Position {
                                line: range.end.line,
                                character: range.end.column,
                            },
                        },
                    },
                    message: related_msg,
                });
            }
        }
    }
    let related_information = if related.is_empty() {
        None
    } else {
        Some(related)
    };
    (message, related_information)
}

/// Find the innermost `SMELT_PATH_CALL` whose text range contains `offset`.
///
/// Used by hover and completion to dispatch on a `smelt.functions.<name>(...)`
/// call site (Phase 48: hover wiring + PASSING-body completion).
pub fn find_smelt_fn_call_at_cursor(
    syntax: &smelt_parser::syntax_kind::SyntaxNode,
    offset: usize,
) -> Option<smelt_parser::ast::SmeltPathCall> {
    let mut best: Option<smelt_parser::ast::SmeltPathCall> = None;
    let mut best_size: usize = usize::MAX;
    for node in syntax.descendants() {
        if let Some(call) = smelt_parser::ast::SmeltPathCall::cast(node) {
            let r = call.text_range();
            let start: usize = r.start().into();
            let end: usize = r.end().into();
            if offset >= start && offset <= end {
                let size = end.saturating_sub(start);
                if size < best_size {
                    best = Some(call);
                    best_size = size;
                }
            }
        }
    }
    best
}

/// Phase 48 — completion helper: resolve the columns of a function-body
/// CTE named `ctx_name` for use in PASSING-body completion. The function
/// body lives in the workspace, so we walk all files to find the
/// signature's declaration and look up the matching CTE in its body.
///
/// Returns an empty vector when the context can't be resolved (function
/// not found, body shape unexpected, no matching CTE).
pub fn passing_body_completion_columns(
    db: &smelt_db::Database,
    workspace: smelt_db::Workspace,
    sig: &smelt_types::FunctionSig,
    ctx_name: &str,
) -> Vec<(String, smelt_types::TypedColumn)> {
    let files: Vec<smelt_db::SourceFile> = workspace.files(db).to_vec();
    for f in files {
        let parse = smelt_db::parse_file(db, f);
        let ast = match smelt_parser::ast::File::cast(parse.syntax()) {
            Some(a) => a,
            None => continue,
        };
        for define in ast.defines() {
            if define.name().as_deref() != Some(sig.name.as_str()) {
                continue;
            }
            let Some(body) = define.body() else { continue };
            // Body may be a SELECT (TableExpr) or a wrapped expression. We
            // only mine CTEs from the SELECT shape.
            let Some(select) = body.select_stmt() else {
                continue;
            };
            let Some(with_clause) = select.with_clause() else {
                continue;
            };
            for cte in with_clause.ctes() {
                if cte.name().as_deref() != Some(ctx_name) {
                    continue;
                }
                // Use a minimal type context — the goal is just to
                // surface column names; type info is best-effort.
                let ctx = smelt_db::TypeContext::new();
                return smelt_db::infer_cte_columns(&cte, &ctx)
                    .into_iter()
                    .filter(|(n, _)| n != "*")
                    .collect();
            }
        }
    }
    Vec::new()
}

/// Phase 48 — completion helper: canonical aggregate function labels for
/// PASSING-body completion when the parameter kind is `Agg` or higher.
/// Scoped down per the plan to a small high-signal set (`COUNT`, `SUM`,
/// `AVG`, `MIN`, `MAX`); the full kind-filtered set is future work.
pub fn passing_body_aggregate_labels() -> Vec<&'static str> {
    vec!["COUNT", "SUM", "AVG", "MIN", "MAX"]
}

// ── Phase 4: LSP hover helpers for list literals and spread ─────────────────
//
// Pure functions over AST + TypeContext. No Salsa calls inside the helper
// bodies — they delegate to `smelt_db::type_inference::infer_list_literal`
// and `smelt_db::type_inference::disambiguate_list_literal`, which are
// themselves pure.

/// Render hover text for a list literal at a position where only the meta-list
/// interpretation is relevant (either explicitly expected or unconstrained).
///
/// Calls `infer_list_literal` and formats the inferred `SmeltType` via
/// `format_smelt_type_hover`. Safe on partially-parsed literals — falls back
/// to `List<Unknown>` rather than panicking.
///
/// Phase A note: not currently called from `Backend::hover`; the dispatch
/// routes through [`hover_text_for_list_literal_dual`] with `expected = None`.
/// Position-aware hover (consulting the splice context to choose the meta vs
/// data-world reading, or to supply the expected sort for an empty literal)
/// lands in Phase B+.
pub fn hover_text_for_list_literal(
    elements: &[smelt_parser::ast::Expr],
    ctx: &smelt_db::TypeContext,
    expected: Option<&smelt_types::signatures::SmeltType>,
) -> String {
    use smelt_db::type_inference::infer_list_literal;
    let result = infer_list_literal(elements, ctx, expected);
    format_smelt_type_hover(&result.inferred)
}

/// Render hover text for a list literal at a position that admits **both**
/// the meta-list and the Data-World array interpretations.
///
/// This occurs when no expected sort is present and the element type is a
/// concrete `Expr<T>`. Both readings are surfaced in the returned string per
/// the spec note "literal accepted in two contexts". Meta wins as the primary
/// reading; the Data-World reading is shown parenthetically.
///
/// Falls back to the single meta-reading when the element type is not a
/// concrete `Expr<Concrete(T)>` (e.g. heterogeneous → `List<Unknown>`).
pub fn hover_text_for_list_literal_dual(
    elements: &[smelt_parser::ast::Expr],
    ctx: &smelt_db::TypeContext,
) -> String {
    use smelt_db::type_inference::infer_list_literal;
    use smelt_types::signatures::{SmeltType, TypeConstraint};

    let result = infer_list_literal(elements, ctx, None);
    let meta_text = format_smelt_type_hover(&result.inferred);

    // Attempt to surface the Data-World array reading.
    // Only emit the dual reading when the inferred type is List<Expr<Concrete(T)>>.
    if let SmeltType::List(inner) = &result.inferred {
        if let SmeltType::Expr(TypeConstraint::Concrete(dt)) = inner.as_ref() {
            let array_text = format!("Array<{dt}>");
            return format!("{meta_text} (or `{array_text}` in array context)");
        }
    }

    // Single reading — meta only (e.g. heterogeneous or nested list).
    meta_text
}

/// Render hover text for a `...expr` spread expression.
///
/// Reads the operand's type. In Phase A, named-variable bindings are not
/// available, so the only operand shape we can fully resolve is a list
/// literal. For non-literal operands the fallback is `List<Unknown>`.
///
/// Safe on partially-parsed spread nodes — returns `List<Unknown>` rather
/// than panicking.
pub fn hover_text_for_list_spread(
    spread: &smelt_parser::ast::ListSpread,
    ctx: &smelt_db::TypeContext,
) -> String {
    use smelt_db::type_inference::infer_list_literal;
    use smelt_types::signatures::SmeltType;

    let fallback = format_smelt_type_hover(&SmeltType::List(Box::new(SmeltType::Unknown)));

    let Some(operand) = spread.operand() else {
        return fallback;
    };

    // If the operand is an array/list literal, infer its type directly.
    if let Some(arr) = operand.as_array_literal() {
        let elems: Vec<smelt_parser::ast::Expr> = arr.elements();
        let result = infer_list_literal(&elems, ctx, None);
        return format_smelt_type_hover(&result.inferred);
    }

    // Non-literal operand — Phase A cannot resolve named-variable types.
    fallback
}

// ============================================================================
// Phase B (meta-language): hover / goto-def / completion pure helpers
//
// These are `pub fn` (not methods) so they can be tested directly from
// the `mod tests` block below without spinning up a full `Backend`.
// ============================================================================

/// Render hover text for a lambda parameter inside a HOF body.
///
/// The `elem_ty` is the element type inferred for the list that the HOF
/// is operating on — this becomes the parameter's bound type.
///
/// Safe on partial parses (body absent, params absent). Returns a
/// description string even when information is incomplete.
pub fn hover_text_for_lambda_param(
    param_name: &str,
    elem_ty: &smelt_types::signatures::SmeltType,
    _lambda: &smelt_parser::ast::Lambda,
    _ctx: &smelt_db::TypeContext,
) -> String {
    let type_str = format_smelt_type_hover(elem_ty);
    format!("**`{param_name}`** (lambda parameter)\n\n`{param_name}: {type_str}`")
}

/// Render hover text for a HOF call (`map`, `filter`, or `reduce`).
///
/// Infers the output type of the HOF call using `infer_hof_call_from_function_call`
/// and formats it via `format_smelt_type_hover`. Safe on partial parses.
pub fn hover_text_for_hof_call(
    call: &smelt_parser::ast::FunctionCall,
    ctx: &smelt_db::TypeContext,
) -> String {
    use smelt_db::type_inference::infer_hof_call_from_function_call;
    let result = infer_hof_call_from_function_call(call, ctx);
    format_smelt_type_hover(&result.inferred)
}

/// Render hover text for a pipe expression `lhs |> rhs(...)`.
///
/// Desugars the pipe to the equivalent direct call and infers its type.
/// Returns the same text as [`hover_text_for_hof_call`] on the equivalent call.
pub fn hover_text_for_pipe_expr(
    pipe: &smelt_parser::ast::PipeExpr,
    ctx: &smelt_db::TypeContext,
) -> String {
    use smelt_db::type_inference::infer_pipe_expr;
    let result = infer_pipe_expr(pipe, ctx, None);
    format_smelt_type_hover(&result.inferred)
}

/// Render hover text for a reducer name in the second-argument position of
/// `reduce(xs, reducer_name)`.
///
/// Shows: input element constraint, output sort, and empty-list identity rule.
/// Returns `"unknown reducer"` for names not in the closed registry.
pub fn hover_text_for_reducer_name(name: &str) -> String {
    use smelt_db::type_inference::{
        EmptyIdentity, ReducerInputConstraint, ReducerOutputSort, REDUCER_REGISTRY,
    };

    let Some(spec) = REDUCER_REGISTRY.iter().find(|r| r.name == name) else {
        return format!("**`{name}`** — unknown reducer");
    };

    let input_desc = match &spec.input_constraint {
        ReducerInputConstraint::AnyExpr => "Expr<T> (any element type)".to_string(),
        ReducerInputConstraint::Boolean => "Expr<Boolean>".to_string(),
        ReducerInputConstraint::Numeric => "Expr<Numeric>".to_string(),
        ReducerInputConstraint::Text => "Expr<Text>".to_string(),
        ReducerInputConstraint::TableExpr => "TableExpr".to_string(),
    };

    let output_desc = match &spec.output_sort {
        ReducerOutputSort::Boolean => "Expr<Boolean>".to_string(),
        ReducerOutputSort::SameAsElementType => "Expr<T> (same as element type)".to_string(),
        ReducerOutputSort::SelectItemsScalar => "SelectItems<Scalar>".to_string(),
        ReducerOutputSort::TableExpr => "TableExpr".to_string(),
    };

    let identity_desc = match &spec.empty_identity {
        EmptyIdentity::Boolean => "TRUE".to_string(),
        EmptyIdentity::Numeric => "0 (cast to element type)".to_string(),
        EmptyIdentity::Text => "''".to_string(),
        EmptyIdentity::EmptySelectItems => "empty SelectItems".to_string(),
        EmptyIdentity::None => "no identity".to_string(),
    };

    format!(
        "**`{name}`** (reducer)\n\n\
         - Input: `{input_desc}`\n\
         - Output: `{output_desc}`\n\
         - Identity: `{identity_desc}`"
    )
}

/// Render hover text for `smelt.config.var('x')`.
///
/// Always shows `Text` as the type. When the variable is present in the
/// `vars:` block of `smelt_yml_text`, also shows the resolved value.
/// When absent, shows a hint that the variable is not declared.
pub fn hover_text_for_config_var(var_name: &str, smelt_yml_text: &str) -> String {
    use smelt_db::config_vars::{coerce_yaml_scalar_to_text, parse_vars_from_yaml};

    let vars = parse_vars_from_yaml(smelt_yml_text).unwrap_or_default();
    match vars.get(var_name) {
        Some(val) => {
            let (text_val, _warn) = coerce_yaml_scalar_to_text(val, var_name);
            format!(
                "**`smelt.config.var('{var_name}')`**\n\n\
                 Type: `Text`\n\n\
                 Resolved value: `'{text_val}'`"
            )
        }
        None => {
            format!(
                "**`smelt.config.var('{var_name}')`**\n\n\
                 Type: `Text`\n\n\
                 Variable `{var_name}` is not declared in `smelt.yml` vars"
            )
        }
    }
}

/// Find the 0-indexed line number of `key:` within the `vars:` block of
/// raw `smelt.yml` text.
///
/// Returns `None` if the key is not found under `vars:`.
/// Used by goto-definition for `smelt.config.var('x')` arguments.
pub fn find_var_line_in_smelt_yml(smelt_yml_text: &str, var_name: &str) -> Option<u32> {
    let mut in_vars = false;
    for (i, line) in smelt_yml_text.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed == "vars:" {
            in_vars = true;
            continue;
        }
        if in_vars {
            // A non-indented non-comment line resets the vars context.
            if !trimmed.is_empty()
                && !trimmed.starts_with('#')
                && !line.starts_with(' ')
                && !line.starts_with('\t')
            {
                in_vars = false;
                continue;
            }
            // Match `  key: value` or `  key:` patterns.
            let key_prefix = format!("{}:", var_name);
            if trimmed.starts_with(&key_prefix) {
                return Some(i as u32);
            }
        }
    }
    None
}

/// Return the name of a HOF function call, handling the case where `filter`
/// is lexed as `FILTER_KW` rather than `IDENT`.
///
/// `FunctionCall::name()` only returns `IDENT`-typed tokens.  Because the
/// lexer emits `filter` as `FILTER_KW`, we fall back to the first
/// non-trivia token's text — mirroring the identical fallback in
/// `smelt_db::type_inference::infer_hof_call_from_function_call_with_expected`.
pub fn hof_call_name(call: &smelt_parser::ast::FunctionCall) -> Option<String> {
    call.name().or_else(|| {
        call.syntax()
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| !t.kind().is_trivia())
            .map(|t| t.text().to_lowercase())
    })
}

/// Return the text range (in the source file) of the binding occurrence of
/// lambda parameter `param_name` in the given lambda node.
///
/// The binding occurrence is the IDENT token inside the `LAMBDA_PARAM_LIST`
/// that bears the parameter name. This is used by goto-definition to jump
/// from a use of the parameter in the body to its binding site.
///
/// Returns `None` when the parameter is not found (e.g., partial parse).
pub fn lambda_param_binder_range(
    lambda: &smelt_parser::ast::Lambda,
    param_name: &str,
) -> Option<smelt_parser::TextRange> {
    use smelt_parser::SyntaxKind;
    let param_list = lambda.param_list()?;
    for child in param_list.syntax().children_with_tokens() {
        // `children_with_tokens` yields `rowan::NodeOrToken` items — match on Token variant.
        if child
            .as_token()
            .map(|t| t.kind() == SyntaxKind::IDENT && t.text() == param_name)
            == Some(true)
        {
            return child.as_token().map(|t| t.text_range());
        }
    }
    None
}

/// Return the parameter names of a lambda for use as completion items.
///
/// The first parameter is the most important — it should appear first in the
/// completion list when the cursor is inside the lambda body.
pub fn lambda_params_for_completion(lambda: &smelt_parser::ast::Lambda) -> Vec<String> {
    lambda.params()
}

/// Return the reducer names from the closed registry that are compatible with
/// the given list type's element type, for use as completion items at the
/// second-argument position of `reduce(xs, _)`.
///
/// When `list_ty` is `None` or `Unknown`, returns the full registry (all names).
/// Otherwise filters by `ReducerInputConstraint::is_satisfied_by` on the element type.
pub fn reducer_completions_for_element_type(
    list_ty: Option<&smelt_types::signatures::SmeltType>,
) -> Vec<String> {
    use smelt_db::type_inference::REDUCER_REGISTRY;
    use smelt_types::signatures::SmeltType;

    let elem_ty: Option<SmeltType> = match list_ty {
        Some(SmeltType::List(inner)) => Some((**inner).clone()),
        _ => None,
    };

    REDUCER_REGISTRY
        .iter()
        .filter(|spec| {
            match &elem_ty {
                Some(et) if !matches!(et, SmeltType::Unknown) => {
                    spec.input_constraint.is_satisfied_by(et)
                }
                // Unknown element type or no list type — offer all reducers.
                _ => true,
            }
        })
        .map(|spec| spec.name.to_string())
        .collect()
}

// ============================================================================
// Phase C (meta-language): hover / goto-def / completion for reflection
//
// Pure helpers — no Salsa calls inside the bodies. `Backend::hover` calls
// them after resolving Salsa-backed data (e.g. ColumnRefValue lists) and
// passes the results in as plain data.
// ============================================================================

/// Render hover text for a `smelt.columns_of(t)` call site.
///
/// - Always shows `List<ColumnRef>` as the return type.
/// - When `columns` is `Some`, also shows the resolved column count and the
///   first five column names (per spec §"LSP support for reflection").
/// - When `columns` is `None` (schema unresolvable or no Salsa context),
///   shows only the type annotation.
///
/// Pure — callers supply the resolved column list.
pub fn hover_text_for_columns_of_call(
    table_name: &str,
    columns: Option<&[smelt_types::signatures::ColumnRefValue]>,
) -> String {
    match columns {
        None => format!(
            "`smelt.columns_of({table_name})`\n\n`List<ColumnRef>`\n\n\
             *Schema not statically resolvable*"
        ),
        Some(cols) => {
            let count = cols.len();
            let preview: Vec<&str> = cols.iter().take(5).map(|c| c.name.as_str()).collect();
            let preview_str = preview.join(", ");
            let ellipsis = if count > 5 { ", …" } else { "" };
            format!(
                "`smelt.columns_of({table_name})`\n\n`List<ColumnRef>`\n\n\
                 {count} columns: {preview_str}{ellipsis}"
            )
        }
    }
}

/// Render hover text for a `ColumnRef`-typed lambda parameter binding.
///
/// Shows `ColumnRef` as the type and lists the three closed fields with their
/// declared types per `COLUMN_REF_FIELDS`.
///
/// Pure — no Salsa dependency.
pub fn hover_text_for_column_ref_binding(param_name: &str) -> String {
    use smelt_types::signatures::COLUMN_REF_FIELDS;
    let mut s = format!(
        "**`{param_name}`** (lambda parameter)\n\n`{param_name}: ColumnRef`\n\n\
         **Fields:**\n"
    );
    for (field_name, field_ty) in COLUMN_REF_FIELDS {
        let ty_str = format_smelt_type_hover(field_ty);
        // Rename `Unknown` to `DataType` for the `type` field so user-facing
        // hover matches the spec description (the field holds a DataType
        // meta-literal; `Unknown` is an internal Phase C placeholder).
        let display_ty = if *field_name == "type" && ty_str == "Unknown" {
            "DataType".to_string()
        } else {
            ty_str
        };
        s.push_str(&format!("- `{field_name}: {display_ty}`\n"));
    }
    s
}

/// Render hover text for a `ColumnRef` field projection `c.<field>`.
///
/// Returns `Some(text)` for the three recognised fields (`name`, `type`,
/// `is_numeric`) and `None` for any other field name (the closed-field
/// invariant — callers should emit `ColumnRefFieldUnknown` in that case).
///
/// Pure — no Salsa dependency.
pub fn hover_text_for_column_ref_field(field_name: &str) -> Option<String> {
    use smelt_types::signatures::column_ref_field;
    let field_ty = column_ref_field(field_name)?;
    let ty_str = format_smelt_type_hover(field_ty);
    // Rename `Unknown` to `DataType` for the `type` field (see
    // `hover_text_for_column_ref_binding` above for the same rationale).
    let display_ty = if field_name == "type" && ty_str == "Unknown" {
        "DataType".to_string()
    } else {
        ty_str
    };
    Some(format!("`{field_name}: {display_ty}` (ColumnRef field)"))
}

/// Render hover text for a meta-`Text` value lifted into an identifier
/// position (one of the four lift positions per spec §"Meta-Text-as-identifier
/// lift").
///
/// - Always describes the lift: `Text → identifier`.
/// - When `resolved_col` is `Some`, also mentions the concrete column name
///   (the `name` field's value at this call site).
///
/// Pure — callers supply the resolved `ColumnRefValue` when available.
pub fn hover_text_for_lifted_identifier(
    lift_expr: &str,
    resolved_col: Option<&smelt_types::signatures::ColumnRefValue>,
) -> String {
    match resolved_col {
        None => format!(
            "`{lift_expr}` — lifted meta-`Text` as identifier\n\n\
             *Concrete value not statically resolvable at this site*"
        ),
        Some(col) => format!(
            "`{lift_expr}` — lifted meta-`Text` as identifier\n\n\
             Resolves to column `{col_name}`",
            col_name = col.name
        ),
    }
}

/// Goto-definition for a `smelt.columns_of` call path — returns `None`
/// (graceful no-op / URL hint per spec §"LSP support for reflection").
///
/// The spec says: *"Goto-definition on `smelt.columns_of` resolves to the
/// reference page (URL hint, graceful no-op when the client lacks support)."*
/// Phase C implements the graceful no-op; a URL-hint extension can land later.
///
/// Pure — no Salsa or Backend dependency.
pub fn goto_def_for_columns_of_call() -> Option<std::path::PathBuf> {
    // Graceful no-op per spec. A future phase may return a URL hint.
    None
}

/// Goto-definition for a lifted meta-`Text` identifier (`c.name` in one of
/// the four lift positions: column-reference, AS-alias, ORDER BY, GROUP BY).
///
/// When the column is statically resolvable (the `ColumnRefValue` carries a
/// `source_span`), resolves to the source column's declaration span.
/// Otherwise returns `None` (graceful no-op — consistent with the spec's
/// "or no-op otherwise" fallback).
///
/// **Known divergence:** Full resolution (tracing the meta-`Text` value
/// through column-resolution and returning the source column's span) requires
/// re-running `columns_of_for_table_expr` with Salsa context.  This pure
/// helper only handles the statically-supplied span case; the Backend-level
/// dispatch wiring is not yet implemented.  Tracked in
/// `docs/plans/20260509-meta-language-overall.md`.
///
/// Pure — no Salsa or Backend dependency.
pub fn goto_def_for_lifted_identifier(
    resolved_col: Option<&smelt_types::signatures::ColumnRefValue>,
) -> Option<std::path::PathBuf> {
    // When the resolved ColumnRefValue carries a source_span (file path +
    // byte offset), we could return that location.  For v1 the source_span
    // field is `Option<TextRange>` (no file path), so we cannot construct a
    // PathBuf from it — return None (graceful no-op).
    let _ = resolved_col; // acknowledged; not yet resolvable to a path
    None
}

/// Return the closed set of `ColumnRef` field names for completion at a
/// field-projection site (`c.<cursor>`).
///
/// Returns exactly `["name", "type", "is_numeric"]` — the three fields in
/// declaration order per `COLUMN_REF_FIELDS`.
///
/// Pure — no Salsa dependency.
pub fn column_ref_field_completions() -> Vec<String> {
    use smelt_types::signatures::COLUMN_REF_FIELDS;
    COLUMN_REF_FIELDS
        .iter()
        .map(|(name, _)| name.to_string())
        .collect()
}

/// Determine whether the text immediately before `cursor_offset` in `sql`
/// ends with `<param_name>.` where `<param_name>` is a lambda parameter
/// that is ColumnRef-typed — i.e. it is bound by a HOF (`map`, `filter`,
/// or `reduce`) whose **first argument** is a `smelt.columns_of(...)` call.
///
/// Returns `Some(param_name)` when the condition holds, `None` otherwise.
///
/// This helper is the single gating function used by both:
///   - the hover dispatch (block 6, field-projection hover), and
///   - the completion dispatch (ColumnRef field completion).
///
/// Pure — no Salsa dependency.
pub fn is_column_ref_param_before_dot(
    file: &smelt_parser::ast::File,
    sql: &str,
    cursor_offset: usize,
) -> Option<String> {
    use smelt_parser::syntax_kind::SyntaxKind;

    let before = &sql[..cursor_offset.min(sql.len())];

    // Quick pre-check: the text before the cursor must end with `<ident>.`
    // (at least one identifier character followed by a dot).  This avoids
    // the more expensive CST walk for unrelated positions.
    let dot_trimmed = before.strip_suffix('.')?;
    let param_name: String = dot_trimmed
        .chars()
        .rev()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    if param_name.is_empty() {
        return None;
    }

    // Find the innermost LAMBDA node that (a) contains cursor_offset and
    // (b) declares `param_name` as one of its parameters.
    let lambda_node = file
        .syntax()
        .descendants()
        .filter(|n| n.kind() == SyntaxKind::LAMBDA)
        .filter(|n| {
            let s: usize = n.text_range().start().into();
            let e: usize = n.text_range().end().into();
            cursor_offset >= s && cursor_offset <= e
        })
        .filter_map(smelt_parser::ast::Lambda::cast)
        .find(|lam| lam.params().iter().any(|p| p == &param_name))?;

    // Walk up from the lambda to find the nearest enclosing HOF FUNCTION_CALL.
    let mut cur = lambda_node.syntax().parent();
    let mut hof_call: Option<smelt_parser::ast::FunctionCall> = None;
    while let Some(node) = cur {
        if node.kind() == SyntaxKind::FUNCTION_CALL {
            if let Some(fc) = smelt_parser::ast::FunctionCall::cast(node.clone()) {
                if matches!(
                    hof_call_name(&fc).as_deref().unwrap_or(""),
                    "map" | "filter" | "reduce"
                ) {
                    hof_call = Some(fc);
                    break;
                }
            }
        }
        cur = node.parent();
    }

    // Check that the HOF's first argument is a `smelt.columns_of(...)` call.
    let hof = hof_call?;
    let first_arg = hof.arguments().into_iter().next()?;
    let path_call = first_arg.as_smelt_path_call()?;
    if path_call.segments() == vec!["columns_of".to_string()] {
        Some(param_name)
    } else {
        None
    }
}

/// Extract in-scope `TableExpr`-valued names from `sql` for use as completion
/// candidates at a `smelt.columns_of(<cursor>)` argument position.
///
/// Two sources are scanned:
///
/// 1. **`smelt.models.<name>` path refs** — any `SMELT_PATH_REF` node with a
///    `models` prefix contributes the model name segment, so
///    `smelt.models.orders` contributes `"orders"`.
///
/// 2. **`smelt.define` `TableExpr`-typed parameters** — any parameter of a
///    `smelt.define` declaration in the same file whose declared type is
///    `TableExpr` contributes its parameter name directly.  This is the
///    primary motivation for `smelt.columns_of` in parametric functions like
///    `smelt.define coalesce_numeric(t: TableExpr) AS (...)` — `t` must be
///    offered when the cursor is at `smelt.columns_of(<cursor>)`.
///
/// Pure — no Salsa dependency; callers can augment the list with Salsa-backed
/// schema information.
pub fn columns_of_arg_completions_for_sql(sql: &str) -> Vec<String> {
    use smelt_parser::ast::{File as AstFile, TypeRefHead};
    let parse = smelt_parser::parse(sql);
    let syntax = parse.syntax();
    let Some(file) = AstFile::cast(syntax) else {
        return Vec::new();
    };

    // Source 1: smelt.models.<name> path refs.
    let mut names: Vec<String> = file
        .syntax()
        .descendants()
        .filter_map(smelt_parser::ast::SmeltPathRef::cast)
        .filter_map(|path_ref| {
            let segs = path_ref.segments();
            // Only `smelt.models.<name>` refs contribute to the list.
            if segs.first().map(|s| s.as_str()) == Some("models") {
                segs.get(1).cloned()
            } else {
                None
            }
        })
        .collect();

    // Source 2: TableExpr-typed parameters of smelt.define declarations.
    for define in file.defines() {
        if let Some(param_list) = define.param_list() {
            for param in param_list.params() {
                if let Some(type_ref) = param.type_ref() {
                    if matches!(type_ref.kind(), TypeRefHead::TableExpr) {
                        if let Some(param_name) = param.name() {
                            if !names.contains(&param_name) {
                                names.push(param_name);
                            }
                        }
                    }
                }
            }
        }
    }

    // Dedup while preserving order (source-1 dedup; source-2 avoids
    // duplicates via the `contains` check above).
    names.dedup();
    names
}

/// Pure dispatch for HOF / lambda / reducer / config-var hover in the
/// meta-language layer.
///
/// Separated from [`Backend::hover`] so it can be tested without spinning up
/// a full tower-lsp `Client` or `Backend`.
///
/// # Dispatch order (most-specific first)
///
/// 1. **Reducer name** — cursor on the second positional argument of a
///    `reduce(xs, name)` call that is a registered reducer.
/// 2. **Lambda parameter** — cursor on a lambda parameter IDENT, either at
///    the binding site (`fn c =>` — the `c` after `fn`) or at any use of
///    that name in the lambda body.
/// 3. **ColumnRef field projection** — cursor on the field token of a
///    `c.<field>` dot expression where `<field>` is a known ColumnRef field
///    AND the receiver is a ColumnRef-typed lambda parameter (HOF first arg
///    is `smelt.columns_of(...)`).  Must run BEFORE block 4 (HOF result type)
///    because the cursor is inside the HOF call range and block 4 would
///    otherwise shadow the field hover.
/// 4. **HOF result type** — cursor anywhere inside a `map(...)` /
///    `filter(...)` / `reduce(...)` call, shows the inferred output type.
/// 5. **`smelt.config.var`** — cursor on a `smelt.config.var('x')` call,
///    shows the resolved value from `smelt_yml_text`.
/// 6. **`smelt.columns_of`** — cursor on the call path of a
///    `smelt.columns_of(t)` call; shows `List<ColumnRef>`.
///
/// Returns `Some(hover_markdown_text)` when a match is found, `None` otherwise.
pub fn hover_text_for_hof_meta_language(
    file: &smelt_parser::ast::File,
    cursor_offset: usize,
    smelt_yml_text: &str,
) -> Option<String> {
    use smelt_parser::syntax_kind::SyntaxKind;

    // ── 1. Reducer name in second arg of reduce(xs, name) ─────────────────
    // Must run BEFORE the HOF result-type check because both match on a
    // `reduce(...)` FUNCTION_CALL node — the reducer-name hover is the
    // more-specific case and must win.
    {
        let reduce_call = file
            .syntax()
            .descendants()
            .filter(|n| n.kind() == SyntaxKind::FUNCTION_CALL)
            .filter(|n| {
                let s: usize = n.text_range().start().into();
                let e: usize = n.text_range().end().into();
                cursor_offset >= s && cursor_offset <= e
            })
            .min_by_key(|n| {
                let s: usize = n.text_range().start().into();
                let e: usize = n.text_range().end().into();
                e - s
            })
            .and_then(smelt_parser::ast::FunctionCall::cast)
            .filter(|c| {
                c.name()
                    .map(|n| n.to_lowercase() == "reduce")
                    .unwrap_or(false)
            });

        if let Some(call) = reduce_call {
            let args = call.arguments();
            if let Some(second_arg) = args.get(1) {
                let arg_start: usize = second_arg.text_range().start().into();
                let arg_end: usize = second_arg.text_range().end().into();
                if cursor_offset >= arg_start && cursor_offset <= arg_end {
                    let reducer_name = second_arg
                        .syntax()
                        .children_with_tokens()
                        .filter_map(|c| c.into_token())
                        .find(|t| t.kind() == SyntaxKind::IDENT)
                        .map(|t| t.text().to_string())
                        .unwrap_or_default();
                    if !reducer_name.is_empty() {
                        return Some(hover_text_for_reducer_name(&reducer_name));
                    }
                }
            }
        }
    }

    // ── 2. Lambda parameter (binder OR body use) ───────────────────────────
    // Must run BEFORE the HOF result-type check because a lambda is nested
    // inside the HOF call node; the HOF check would otherwise shadow it.
    {
        let lambda_node = file
            .syntax()
            .descendants()
            .filter(|n| n.kind() == SyntaxKind::LAMBDA)
            .filter(|n| {
                let s: usize = n.text_range().start().into();
                let e: usize = n.text_range().end().into();
                cursor_offset >= s && cursor_offset <= e
            })
            .min_by_key(|n| {
                let s: usize = n.text_range().start().into();
                let e: usize = n.text_range().end().into();
                e - s
            });

        if let Some(ln) = lambda_node {
            if let Some(lambda) = smelt_parser::ast::Lambda::cast(ln.clone()) {
                let params = lambda.params();
                for param_name in &params {
                    // Check binder position.
                    let on_binder = lambda_param_binder_range(&lambda, param_name)
                        .map(|r| {
                            let s: usize = r.start().into();
                            let e: usize = r.end().into();
                            cursor_offset >= s && cursor_offset <= e
                        })
                        .unwrap_or(false);

                    // Check uses in the lambda body: any IDENT token with
                    // the same name that sits inside the body subtree.
                    let on_body_use = lambda.body().is_some_and(|body| {
                        body.syntax()
                            .descendants_with_tokens()
                            .filter_map(|e| e.into_token())
                            .filter(|t| {
                                t.kind() == SyntaxKind::IDENT && t.text() == param_name.as_str()
                            })
                            .any(|t| {
                                let s: usize = t.text_range().start().into();
                                let e: usize = t.text_range().end().into();
                                cursor_offset >= s && cursor_offset <= e
                            })
                    });

                    if on_binder || on_body_use {
                        // Infer the element type from the enclosing HOF call.
                        // Walk up through any intermediate nodes (EXPRESSION,
                        // ARG_LIST, etc.) until we find a FUNCTION_CALL ancestor.
                        let hof_call = {
                            let mut cur = ln.parent();
                            let mut found = None;
                            while let Some(node) = cur {
                                if node.kind() == SyntaxKind::FUNCTION_CALL {
                                    found = smelt_parser::ast::FunctionCall::cast(node.clone())
                                        .filter(|c| {
                                            matches!(
                                                hof_call_name(c).as_deref().unwrap_or(""),
                                                "map" | "filter" | "reduce"
                                            )
                                        });
                                    break;
                                }
                                cur = node.parent();
                            }
                            found
                        };
                        use smelt_types::signatures::SmeltType;
                        // Infer the INPUT element type from the HOF's first
                        // argument (the list being iterated).  Using the HOF
                        // result type would be wrong for `map`, which returns
                        // `List<U>` (the OUTPUT type); we want `T` from
                        // `List<T>`.  `filter` preserves the element type so
                        // the result would coincidentally be correct, but
                        // deriving it from the first arg is more principled and
                        // correct for all three HOFs.
                        let elem_ty = hof_call
                            .as_ref()
                            .and_then(|c| {
                                let args = c.arguments();
                                let first_arg = args.first()?;
                                let ctx = smelt_db::TypeContext::new();
                                // If the first arg is an array literal, use
                                // `infer_list_literal` to get `List<T>` then
                                // extract `T`.
                                let list_ty = if let Some(arr) = first_arg.as_array_literal() {
                                    let elems: Vec<_> = arr.elements();
                                    smelt_db::type_inference::infer_list_literal(&elems, &ctx, None)
                                        .inferred
                                } else {
                                    // Non-literal first argument.
                                    // Special case: recognise wide-reflection and
                                    // narrow-reflection call sites so the lambda
                                    // element type can be determined statically.
                                    // - `smelt.columns_of(t)` → `List<ColumnRef>`
                                    // - `smelt.models.with_tag(t)` / `smelt.models.all` → `List<ModelRef>`
                                    // - `smelt.sources.with_tag(t)` / `smelt.sources.all` → `List<SourceRef>`
                                    if let Some(path_call) = first_arg.as_smelt_path_call() {
                                        let segs = path_call.segments();
                                        if segs == vec!["columns_of".to_string()] {
                                            SmeltType::List(Box::new(SmeltType::ColumnRef))
                                        } else if segs.first().map(|s| s.as_str()) == Some("models")
                                            && segs
                                                .get(1)
                                                .map(|s| s.as_str() == "with_tag" || s.as_str() == "all")
                                                .unwrap_or(false)
                                        {
                                            SmeltType::List(Box::new(SmeltType::ModelRef))
                                        } else if segs.first().map(|s| s.as_str()) == Some("sources")
                                            && segs
                                                .get(1)
                                                .map(|s| s.as_str() == "with_tag" || s.as_str() == "all")
                                                .unwrap_or(false)
                                        {
                                            SmeltType::List(Box::new(SmeltType::SourceRef))
                                        } else {
                                            SmeltType::Unknown
                                        }
                                    } else {
                                        // Fall back to the full HOF inference path.
                                        // For `filter`, the result IS `List<T>`.
                                        // For `map`/`reduce`, result type is different;
                                        // we accept Unknown for those non-literal cases.
                                        let result =
                                            smelt_db::type_inference::infer_hof_call_from_function_call(
                                                c, &ctx,
                                            );
                                        let hof_name = hof_call_name(c).unwrap_or_default();
                                        if hof_name == "filter" {
                                            result.inferred
                                        } else {
                                            SmeltType::Unknown
                                        }
                                    }
                                };
                                if let SmeltType::List(inner) = list_ty {
                                    Some(*inner)
                                } else {
                                    None
                                }
                            })
                            .unwrap_or(SmeltType::Unknown);
                        // ColumnRef/ModelRef/SourceRef-typed lambda parameter:
                        // use the specialised binding helper (shows the closed
                        // field list) instead of the generic lambda-param
                        // helper.
                        if matches!(elem_ty, SmeltType::ColumnRef) {
                            return Some(hover_text_for_column_ref_binding(param_name));
                        }
                        if matches!(elem_ty, SmeltType::ModelRef) {
                            return Some(hover_text_for_model_ref_binding(param_name));
                        }
                        if matches!(elem_ty, SmeltType::SourceRef) {
                            return Some(hover_text_for_source_ref_binding(param_name));
                        }
                        let ctx = smelt_db::TypeContext::new();
                        return Some(hover_text_for_lambda_param(
                            param_name, &elem_ty, &lambda, &ctx,
                        ));
                    }
                }
            }
        }
    }

    // ── 3. ColumnRef / ModelRef / SourceRef field projection ─────────────────
    // Cursor on `<field>` in `c.<field>` / `m.<field>` / `s.<field>`.
    // Runs BEFORE the HOF result-type block so field hover wins over outer HOF.
    // Receiver checks ensure plain SQL field access does NOT trigger this hover.
    {
        use smelt_parser::SyntaxKind::{DOT, IDENT};
        let syntax_node = file.syntax();
        let tokens: Vec<_> = syntax_node
            .descendants_with_tokens()
            .filter_map(|e| e.into_token())
            .collect();
        let file_sql = file.syntax().text().to_string();
        for (i, tok) in tokens.iter().enumerate() {
            if tok.kind() == DOT {
                if let Some(next_tok) = tokens.get(i + 1) {
                    if next_tok.kind() == IDENT {
                        let start: usize = next_tok.text_range().start().into();
                        let end: usize = next_tok.text_range().end().into();
                        if cursor_offset >= start && cursor_offset <= end {
                            let field_name = next_tok.text();
                            let dot_end: usize = tok.text_range().end().into();
                            // ColumnRef: HOF first arg is smelt.columns_of(...)
                            if is_column_ref_param_before_dot(file, &file_sql, dot_end).is_some() {
                                if let Some(hover_text) =
                                    hover_text_for_column_ref_field(field_name)
                                {
                                    return Some(hover_text);
                                }
                            }
                            // ModelRef: HOF first arg is smelt.models.with_tag/all
                            if is_model_ref_param_before_dot(file, &file_sql, dot_end).is_some() {
                                if let Some(hover_text) = hover_text_for_model_ref_field(field_name)
                                {
                                    return Some(hover_text);
                                }
                            }
                            // SourceRef: HOF first arg is smelt.sources.with_tag/all
                            if is_source_ref_param_before_dot(file, &file_sql, dot_end).is_some() {
                                if let Some(hover_text) =
                                    hover_text_for_source_ref_field(field_name)
                                {
                                    return Some(hover_text);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // ── 4. smelt.models.* / smelt.sources.* wide-reflection accessor call ─────
    // Must run BEFORE the HOF result-type check.  When the cursor is inside
    // `reduce(smelt.models.all(), union_all)`, the cursor position is inside
    // the `smelt.models.all()` SmeltPathCall node (a more-specific match).
    // The HOF result-type block would otherwise intercept the position first.
    {
        let wide_call = file
            .syntax()
            .descendants()
            .filter_map(smelt_parser::ast::SmeltPathCall::cast)
            .find(|c| {
                let segs = c.segments();
                let first = segs.first().map(|s| s.as_str());
                let second = segs.get(1).map(|s| s.as_str());
                let is_wide = (first == Some("models") || first == Some("sources"))
                    && (second == Some("with_tag") || second == Some("all"));
                if !is_wide {
                    return false;
                }
                let r = c.text_range();
                let s: usize = r.start().into();
                let e: usize = r.end().into();
                cursor_offset >= s && cursor_offset <= e
            });

        if let Some(call) = wide_call {
            let segs = call.segments();
            let namespace = segs.first().map(|s| s.as_str()).unwrap_or("models");
            let accessor = segs.get(1).map(|s| s.as_str()).unwrap_or("all");
            if namespace == "models" {
                if accessor == "with_tag" {
                    let tag = call
                        .arg_list()
                        .and_then(|al| al.positional_args().into_iter().next())
                        .map(|a| {
                            let t = a.text();
                            t.trim_matches('\'').trim_matches('"').to_string()
                        })
                        .unwrap_or_default();
                    return Some(hover_text_for_models_with_tag_call(&tag, None));
                } else {
                    return Some(hover_text_for_models_all(None));
                }
            } else {
                if accessor == "with_tag" {
                    let tag = call
                        .arg_list()
                        .and_then(|al| al.positional_args().into_iter().next())
                        .map(|a| {
                            let t = a.text();
                            t.trim_matches('\'').trim_matches('"').to_string()
                        })
                        .unwrap_or_default();
                    return Some(hover_text_for_sources_with_tag_call(&tag, None));
                } else {
                    return Some(hover_text_for_sources_all(None));
                }
            }
        }
    }

    // ── 5. HOF result type (map / filter / reduce) ─────────────────────────
    {
        let hof_call = file
            .syntax()
            .descendants()
            .filter(|n| n.kind() == SyntaxKind::FUNCTION_CALL)
            .filter(|n| {
                let s: usize = n.text_range().start().into();
                let e: usize = n.text_range().end().into();
                cursor_offset >= s && cursor_offset <= e
            })
            .min_by_key(|n| {
                let s: usize = n.text_range().start().into();
                let e: usize = n.text_range().end().into();
                e - s
            })
            .and_then(smelt_parser::ast::FunctionCall::cast)
            .filter(|c| {
                matches!(
                    hof_call_name(c).as_deref().unwrap_or(""),
                    "map" | "filter" | "reduce"
                )
            });

        if let Some(call) = hof_call {
            let ctx = smelt_db::TypeContext::new();
            return Some(hover_text_for_hof_call(&call, &ctx));
        }
    }

    // ── 5. smelt.config.var('x') ───────────────────────────────────────────
    {
        let config_call = file
            .syntax()
            .descendants()
            .filter(|n| n.kind() == SyntaxKind::FUNCTION_CALL)
            .filter(|n| {
                let s: usize = n.text_range().start().into();
                let e: usize = n.text_range().end().into();
                cursor_offset >= s && cursor_offset <= e
            })
            .min_by_key(|n| {
                let s: usize = n.text_range().start().into();
                let e: usize = n.text_range().end().into();
                e - s
            })
            .and_then(smelt_parser::ast::FunctionCall::cast)
            .filter(|c| {
                let raw = c
                    .syntax()
                    .children_with_tokens()
                    .filter_map(|e| e.into_token())
                    .find(|t| t.kind() == SyntaxKind::IDENT)
                    .map(|t| t.text().to_string())
                    .unwrap_or_default();
                raw == "smelt"
                    || (c.namespace().as_deref() == Some("smelt")
                        && c.name().as_deref() == Some("config"))
            });

        if let Some(call) = config_call {
            let full_text = call.text();
            if full_text.contains("config.var") || full_text.contains("config") {
                let var_call = call
                    .syntax()
                    .descendants()
                    .filter_map(smelt_parser::ast::FunctionCall::cast)
                    .find(|c| c.name().as_deref() == Some("var"));
                if let Some(vc) = var_call {
                    let args = vc.arguments();
                    if let Some(arg) = args.first() {
                        if smelt_db::config_vars::is_string_literal_expr(arg) {
                            if let Some(var_name) =
                                smelt_db::config_vars::extract_string_literal_value(arg)
                            {
                                return Some(hover_text_for_config_var(&var_name, smelt_yml_text));
                            }
                        }
                    }
                }
            }
        }
    }

    // ── 6. smelt.columns_of(t) — cursor on the call path ─────────────────────
    // The generic HOF check only matches `map`/`filter`/`reduce`, so this
    // block runs without conflict.  We check for a SMELT_PATH_CALL whose
    // segments are `["columns_of"]` (the leading `smelt` is implicit).
    {
        let columns_of_call = file
            .syntax()
            .descendants()
            .filter_map(smelt_parser::ast::SmeltPathCall::cast)
            .filter(|c| {
                let segs = c.segments();
                segs == vec!["columns_of".to_string()]
            })
            .find(|c| {
                let r = c.text_range();
                let s: usize = r.start().into();
                let e: usize = r.end().into();
                cursor_offset >= s && cursor_offset <= e
            });

        if let Some(call) = columns_of_call {
            // Extract the table name argument (first positional arg).
            let table_name = call
                .arg_list()
                .and_then(|al| al.positional_args().into_iter().next())
                .map(|a| a.text())
                .unwrap_or_else(|| "?".to_string());
            // Return hover with no resolved columns (no Salsa context in this
            // pure dispatch). Backend::hover supplies resolved columns via its
            // own block; this pure helper is exercised by the dispatch tests.
            return Some(hover_text_for_columns_of_call(&table_name, None));
        }
    }

    None
}

// ============================================================================
// Phase D (meta-language): hover / goto-def / completion for wide reflection
//
// Pure helpers — no Salsa calls inside the bodies.  Backend handlers call them
// after resolving Salsa-backed data (e.g. ModelRefValue lists) and pass the
// results in as plain data.
// ============================================================================

/// Render hover text for a `smelt.models.with_tag(t)` call site.
///
/// - Always shows `List<ModelRef>` as the return type.
/// - When `models` is `Some`, also shows the resolved match count and the
///   first five matching model names (per spec §"LSP support for wide
///   reflection").
/// - When `models` is `None` (workspace not resolvable at this cursor),
///   shows only the type annotation.
///
/// Pure — callers supply the resolved model list.
pub fn hover_text_for_models_with_tag_call(
    tag: &str,
    models: Option<&[smelt_types::signatures::ModelRefValue]>,
) -> String {
    match models {
        None => format!(
            "`smelt.models.with_tag('{tag}')`\n\n`List<ModelRef>`\n\n\
             *Workspace not statically resolvable*"
        ),
        Some(ms) => {
            let count = ms.len();
            let preview: Vec<&str> = ms.iter().take(5).map(|m| m.name.as_str()).collect();
            let preview_str = preview.join(", ");
            let ellipsis = if count > 5 { ", …" } else { "" };
            format!(
                "`smelt.models.with_tag('{tag}')`\n\n`List<ModelRef>`\n\n\
                 {count} matching models: {preview_str}{ellipsis}"
            )
        }
    }
}

/// Render hover text for a `smelt.sources.with_tag(t)` call site.
///
/// Mirrors `hover_text_for_models_with_tag_call` but for `SourceRef`.
///
/// Pure — callers supply the resolved source list.
pub fn hover_text_for_sources_with_tag_call(
    tag: &str,
    sources: Option<&[smelt_types::signatures::SourceRefValue]>,
) -> String {
    match sources {
        None => format!(
            "`smelt.sources.with_tag('{tag}')`\n\n`List<SourceRef>`\n\n\
             *Workspace not statically resolvable*"
        ),
        Some(ss) => {
            let count = ss.len();
            let preview: Vec<&str> = ss.iter().take(5).map(|s| s.name.as_str()).collect();
            let preview_str = preview.join(", ");
            let ellipsis = if count > 5 { ", …" } else { "" };
            format!(
                "`smelt.sources.with_tag('{tag}')`\n\n`List<SourceRef>`\n\n\
                 {count} matching sources: {preview_str}{ellipsis}"
            )
        }
    }
}

/// Render hover text for `smelt.models.all` call / accessor.
///
/// - Always shows the signature `() -> List<ModelRef>`.
/// - When `total` is `Some`, also shows the workspace's total model count.
///
/// Pure — callers supply the total count.
pub fn hover_text_for_models_all(total: Option<usize>) -> String {
    match total {
        None => "`smelt.models.all`\n\n`() -> List<ModelRef>`\n\n\
                 *Workspace not statically resolvable*"
            .to_string(),
        Some(n) => format!(
            "`smelt.models.all`\n\n`() -> List<ModelRef>`\n\n\
             {n} models in workspace"
        ),
    }
}

/// Render hover text for `smelt.sources.all` call / accessor.
///
/// Mirrors `hover_text_for_models_all` but for `SourceRef`.
///
/// Pure — callers supply the total count.
pub fn hover_text_for_sources_all(total: Option<usize>) -> String {
    match total {
        None => "`smelt.sources.all`\n\n`() -> List<SourceRef>`\n\n\
                 *Workspace not statically resolvable*"
            .to_string(),
        Some(n) => format!(
            "`smelt.sources.all`\n\n`() -> List<SourceRef>`\n\n\
             {n} sources in workspace"
        ),
    }
}

/// Render hover text for a `ModelRef`-typed lambda parameter binding.
///
/// Shows `ModelRef` as the type and lists the four closed fields with their
/// declared types per `MODEL_REF_FIELDS`.
///
/// Pure — no Salsa dependency.
pub fn hover_text_for_model_ref_binding(param_name: &str) -> String {
    use smelt_types::signatures::MODEL_REF_FIELDS;
    let mut s = format!(
        "**`{param_name}`** (lambda parameter)\n\n`{param_name}: ModelRef`\n\n\
         **Fields:**\n"
    );
    for (field_name, field_ty) in MODEL_REF_FIELDS.iter() {
        let ty_str = format_smelt_type_hover(field_ty);
        s.push_str(&format!("- `{field_name}: {ty_str}`\n"));
    }
    s
}

/// Render hover text for a `SourceRef`-typed lambda parameter binding.
///
/// Shows `SourceRef` as the type and lists the four closed fields with their
/// declared types per `SOURCE_REF_FIELDS`.
///
/// Pure — no Salsa dependency.
pub fn hover_text_for_source_ref_binding(param_name: &str) -> String {
    use smelt_types::signatures::SOURCE_REF_FIELDS;
    let mut s = format!(
        "**`{param_name}`** (lambda parameter)\n\n`{param_name}: SourceRef`\n\n\
         **Fields:**\n"
    );
    for (field_name, field_ty) in SOURCE_REF_FIELDS.iter() {
        let ty_str = format_smelt_type_hover(field_ty);
        s.push_str(&format!("- `{field_name}: {ty_str}`\n"));
    }
    s
}

/// Render hover text for a `ModelRef` field projection `m.<field>`.
///
/// Returns `Some(text)` for the four recognised fields (`path`, `name`, `tags`,
/// `columns`) and `None` for any other field name.
///
/// Pure — no Salsa dependency.
pub fn hover_text_for_model_ref_field(field_name: &str) -> Option<String> {
    use smelt_types::signatures::model_ref_field;
    let field_ty = model_ref_field(field_name)?;
    let ty_str = format_smelt_type_hover(field_ty);
    Some(format!("`{field_name}: {ty_str}` (ModelRef field)"))
}

/// Render hover text for a `SourceRef` field projection `s.<field>`.
///
/// Returns `Some(text)` for the four recognised fields (`path`, `name`, `tags`,
/// `columns`) and `None` for any other field name.
///
/// Pure — no Salsa dependency.
pub fn hover_text_for_source_ref_field(field_name: &str) -> Option<String> {
    use smelt_types::signatures::source_ref_field;
    let field_ty = source_ref_field(field_name)?;
    let ty_str = format_smelt_type_hover(field_ty);
    Some(format!("`{field_name}: {ty_str}` (SourceRef field)"))
}

/// Goto-definition for `smelt.models.*` / `smelt.sources.*` accessor call paths —
/// returns `None` (graceful no-op / URL hint per spec §"LSP support for wide
/// reflection").
///
/// Pure — no Salsa or Backend dependency.
pub fn goto_def_for_wide_reflection_accessor() -> Option<std::path::PathBuf> {
    // Graceful no-op per spec. A future phase may return a URL hint.
    None
}

/// Goto-definition from a `ModelRef`-typed value at a splice site or from a
/// `ModelRef` field projection (`m.path`, `m.name`) — resolves to the model's
/// source `.sql` file.
///
/// When `source_path` is `Some`, returns the path. Otherwise returns `None`
/// (graceful no-op).
///
/// Pure — callers supply the resolved path.
pub fn goto_def_for_model_ref_value(
    source_path: Option<std::path::PathBuf>,
) -> Option<std::path::PathBuf> {
    source_path
}

/// Goto-definition from a `SourceRef`-typed value — resolves to the source
/// YAML file.
///
/// When `yaml_path` is `Some`, returns the path. Otherwise returns `None`
/// (graceful no-op).
///
/// Pure — callers supply the resolved path.
pub fn goto_def_for_source_ref_value(
    yaml_path: Option<std::path::PathBuf>,
) -> Option<std::path::PathBuf> {
    yaml_path
}

/// Return the closed set of accessor names for `smelt.models.<cursor>` or
/// `smelt.sources.<cursor>` completion.
///
/// Returns exactly `["with_tag", "all"]` — the two accessors per spec.
///
/// Pure — no Salsa dependency.
pub fn wide_reflection_accessor_completions() -> Vec<String> {
    vec!["with_tag".to_string(), "all".to_string()]
}

/// Return the closed set of `ModelRef` field names for completion at a
/// field-projection site (`m.<cursor>`).
///
/// Returns exactly `["path", "name", "tags", "columns"]` — the four fields in
/// declaration order per `MODEL_REF_FIELDS`.
///
/// Pure — no Salsa dependency.
pub fn model_ref_field_completions() -> Vec<String> {
    use smelt_types::signatures::MODEL_REF_FIELDS;
    MODEL_REF_FIELDS
        .iter()
        .map(|(name, _)| name.to_string())
        .collect()
}

/// Return the closed set of `SourceRef` field names for completion at a
/// field-projection site (`s.<cursor>`).
///
/// Returns exactly `["path", "name", "tags", "columns"]` — the four fields in
/// declaration order per `SOURCE_REF_FIELDS`.
///
/// Pure — no Salsa dependency.
pub fn source_ref_field_completions() -> Vec<String> {
    use smelt_types::signatures::SOURCE_REF_FIELDS;
    SOURCE_REF_FIELDS
        .iter()
        .map(|(name, _)| name.to_string())
        .collect()
}

// ============================================================================
// Phase E1: Record hover / goto-def / completion helpers
// ============================================================================

/// Render hover text for a `smelt.record` declaration name token.
///
/// Shows the closed field list with types and the declaration file path.
///
/// Pure — callers supply the fields map and file path.
pub fn hover_text_for_record_decl_name(
    type_name: &str,
    fields: &std::collections::BTreeMap<String, smelt_types::signatures::SmeltType>,
    file_path: &str,
) -> String {
    let field_lines: Vec<String> = fields
        .iter()
        .map(|(k, v)| format!("  {k}: {}", smelt_types::format_smelt_type_hover(v)))
        .collect();
    format!(
        "`smelt.record {type_name}`\n\n```\n{{\n{}\n}}\n```\n\n*Declared in `{file_path}`*",
        field_lines.join(",\n")
    )
}

/// Render hover text for a record-typed binding (parameter or variable).
///
/// Shows the record name and its closed field list.
///
/// Pure — callers supply the resolved type.
pub fn hover_text_for_record_typed_binding(
    param_name: &str,
    ty: &smelt_types::signatures::SmeltType,
) -> String {
    use smelt_types::format_smelt_type_hover;
    let type_str = format_smelt_type_hover(ty);
    match ty {
        smelt_types::signatures::SmeltType::Record { fields, .. } => {
            let field_lines: Vec<String> = fields
                .iter()
                .map(|(k, v)| format!("  {k}: {}", format_smelt_type_hover(v)))
                .collect();
            format!(
                "`{param_name}: {type_str}`\n\n```\n{{\n{}\n}}\n```",
                field_lines.join(",\n")
            )
        }
        _ => format!("`{param_name}: {type_str}`"),
    }
}

/// Render hover text for a record field projection (`.fieldname`).
///
/// Returns `Some(text)` when the field is found in the record's declared field
/// set, `None` otherwise (caller should fall through to the next hover handler).
///
/// Pure — callers supply the record type.
pub fn hover_text_for_record_field_projection(
    field_name: &str,
    record_ty: &smelt_types::signatures::SmeltType,
) -> Option<String> {
    use smelt_types::format_smelt_type_hover;
    match record_ty {
        smelt_types::signatures::SmeltType::Record { fields, name } => {
            let field_ty = fields.get(field_name)?;
            let type_str = format_smelt_type_hover(field_ty);
            let record_name = name.as_deref().unwrap_or("{record}");
            Some(format!("`{record_name}.{field_name}: {type_str}`"))
        }
        _ => None,
    }
}

/// Render hover text for a record literal opening brace — shows the inferred
/// target record type when statically known.
///
/// Pure — callers supply the inferred / target type.
pub fn hover_text_for_record_literal_brace(ty: &smelt_types::signatures::SmeltType) -> String {
    use smelt_types::format_smelt_type_hover;
    format!("`{}`", format_smelt_type_hover(ty))
}

/// Return the closed set of field names plus their declared type detail for
/// completion at a record literal field-key position, excluding fields already
/// filled.
///
/// Each entry is `(field_name, formatted_type)` so the backend can surface the
/// declared type as the completion-item detail per spec §"LSP support for
/// records".
///
/// Pure — callers supply the declared `(name, type)` pairs and the
/// already-filled names.
pub fn record_literal_field_completions(
    declared: &[(String, smelt_types::signatures::SmeltType)],
    already_filled: &[String],
) -> Vec<(String, String)> {
    use smelt_types::format_smelt_type_hover;
    declared
        .iter()
        .filter(|(name, _)| !already_filled.contains(name))
        .map(|(name, ty)| (name.clone(), format_smelt_type_hover(ty)))
        .collect()
}

/// Return the closed set of field names plus their declared type detail for
/// completion at a record field-projection site (`r.<cursor>`).
///
/// Each entry is `(field_name, formatted_type)` so the backend can surface the
/// declared type as the completion-item detail per spec §"LSP support for
/// records".
///
/// Pure — callers supply the declared field map.
pub fn record_field_projection_completions(
    fields: &std::collections::BTreeMap<String, smelt_types::signatures::SmeltType>,
) -> Vec<(String, String)> {
    use smelt_types::format_smelt_type_hover;
    fields
        .iter()
        .map(|(name, ty)| (name.clone(), format_smelt_type_hover(ty)))
        .collect()
}

// ============================================================================
// Phase E1: Map hover / goto-def / completion helpers
// ============================================================================

/// The closed set of Map API method names per spec.
pub const MAP_API_METHODS: &[(&str, &str)] = &[
    ("entries", "Map<K, V> -> List<{key: K, value: V}>"),
    ("keys", "Map<K, V> -> List<K>"),
    ("values", "Map<K, V> -> List<V>"),
    ("get", "(Map<K, V>, K) -> V"),
    ("has", "(Map<K, V>, K) -> Boolean"),
];

/// Render hover text for a `Map<K, V>`-typed binding.
///
/// Shows the type, entry count (when resolved), and first five keys (when
/// statically known).
///
/// Pure — callers supply the resolved value summary.
pub fn hover_text_for_map_typed_binding(
    param_name: &str,
    key_ty: &smelt_types::signatures::SmeltType,
    val_ty: &smelt_types::signatures::SmeltType,
    entry_count: Option<usize>,
    first_keys: Option<&[String]>,
) -> String {
    use smelt_types::format_smelt_type_hover;
    let type_str = format!(
        "Map<{}, {}>",
        format_smelt_type_hover(key_ty),
        format_smelt_type_hover(val_ty)
    );
    let mut text = format!("`{param_name}: {type_str}`");
    if let Some(count) = entry_count {
        text.push_str(&format!("\n\n{count} entries"));
        if let Some(keys) = first_keys {
            let preview: Vec<&str> = keys.iter().take(5).map(|k| k.as_str()).collect();
            let ellipsis = if keys.len() > 5 { ", …" } else { "" };
            text.push_str(&format!(": {}{ellipsis}", preview.join(", ")));
        }
    }
    text
}

/// Render hover text for a Map method call site.
///
/// Shows the method signature and, when statically resolvable, a resolution
/// hint appropriate to the method:
///
/// - `entries`, `keys`, `values` — render the length of the resulting list via
///   `resolved_len`.
/// - `get` — render the concrete bound value (when the key is statically known
///   and present) via `resolved_value`.
/// - `has` — render the boolean result via `resolved_value`.
///
/// Pure — callers supply the method name, key/val types, optional resolved
/// length, and optional resolved value summary.
pub fn hover_text_for_map_method_call(
    method: &str,
    key_ty: &smelt_types::signatures::SmeltType,
    val_ty: &smelt_types::signatures::SmeltType,
    resolved_len: Option<usize>,
    resolved_value: Option<&str>,
) -> String {
    use smelt_types::format_smelt_type_hover;
    let signature = MAP_API_METHODS
        .iter()
        .find(|(n, _)| *n == method)
        .map(|(_, sig)| *sig)
        .unwrap_or("Map<K, V> -> ?");
    let k = format_smelt_type_hover(key_ty);
    let v = format_smelt_type_hover(val_ty);
    // Specialise the generic signature to the concrete types.
    let concrete_sig = signature
        .replace("Map<K, V>", &format!("Map<{k}, {v}>"))
        .replace("<K, ", &format!("<{k}, "))
        .replace(", V>", &format!(", {v}>"))
        .replace(", V}", &format!(", {v}}}"))
        .replace("-> V", &format!("-> {v}"))
        .replace(", K)", &format!(", {k})"))
        .replace(", K> ", &format!(", {k}> "));
    let mut text = format!("`m.{method}()`\n\n`{concrete_sig}`");
    match method {
        "get" | "has" => {
            if let Some(value) = resolved_value {
                text.push_str(&format!("\n\n*Resolved: {value}*"));
            }
        }
        _ => {
            if let Some(len) = resolved_len {
                text.push_str(&format!("\n\n*Resolved: {len} entries*"));
            }
        }
    }
    text
}

/// Return the closed set of Map API method names for completion at `m.<cursor>`.
///
/// Returns exactly `["entries", "keys", "values", "get", "has"]`.
///
/// Pure — no Salsa dependency.
pub fn map_api_method_completions() -> Vec<String> {
    MAP_API_METHODS.iter().map(|(n, _)| n.to_string()).collect()
}

/// Return statically-known keys for `m.get(<cursor>)` / `m.has(<cursor>)`.
///
/// Returns `None` when the map is not statically resolvable.
///
/// Pure — callers supply the resolved key list.
pub fn map_get_key_completions(known_keys: Option<&[String]>) -> Vec<String> {
    known_keys.unwrap_or(&[]).to_vec()
}

// ============================================================================
// Phase E1: Loader hover / goto-def / completion helpers
// ============================================================================

/// Render hover text for a `smelt.config.load_yaml` / `load_json` call site.
///
/// Shows the resolved path, row count (for `List`/`Map` schemas) or field set
/// (for record schemas), and optionally the last-modified timestamp.
///
/// Pure — callers supply the resolved summary.
pub fn hover_text_for_loader_call(
    loader_fn: &str,
    resolved_path: &str,
    schema_summary: &str,
    last_modified: Option<&str>,
) -> String {
    let mut text = format!(
        "`smelt.config.{loader_fn}('{resolved_path}', …)`\n\n\
         **Path:** `{resolved_path}`\n\n\
         **Schema:** {schema_summary}"
    );
    if let Some(ts) = last_modified {
        text.push_str(&format!("\n\n*Last modified: {ts}*"));
    }
    text
}

/// Return filesystem path completion candidates for a loader's `path` argument.
///
/// Filters by extension (`yaml`/`yml` for `load_yaml`, `json` for `load_json`)
/// and returns workspace-relative paths.
///
/// Pure — callers supply the list of candidate paths.
pub fn loader_path_completions(candidates: &[String], format: &str) -> Vec<String> {
    let ext = match format {
        "yaml" => vec!["yaml", "yml"],
        "json" => vec!["json"],
        _ => vec![],
    };
    candidates
        .iter()
        .filter(|p| ext.iter().any(|e| p.ends_with(&format!(".{e}"))))
        .cloned()
        .collect()
}

/// Return in-scope `smelt.record` names for completion at a loader's schema argument.
///
/// Also includes an inline stub entry `{…}`.
///
/// Pure — callers supply the registered record names.
pub fn loader_schema_completions(record_names: &[String]) -> Vec<String> {
    let mut result: Vec<String> = record_names.to_vec();
    result.push("{…}".to_string());
    result
}

// ============================================================================
// Phase E1: goto-def pure helpers (record names, fields, loader path, YAML row)
// ============================================================================

/// Goto-definition on a `smelt.record` name reference — resolves to the
/// declaration site (file + range).
///
/// Returns `Some(Location)` pointing at `decl_path:decl_range`.
///
/// Per spec §"LSP support for records": *"Goto-definition on a `smelt.record`
/// name reference resolves to the declaration site (file + range)."*
///
/// Pure — callers supply the pre-resolved declaration path and range.
pub fn goto_def_for_smelt_record_name(
    decl_path: &std::path::Path,
    decl_range: Range,
) -> Option<Location> {
    let uri = Url::from_file_path(decl_path).ok()?;
    Some(Location {
        uri,
        range: decl_range,
    })
}

/// Goto-definition on a record literal's field name — resolves to the declared
/// field's source span in the corresponding record type.
///
/// Returns `Some(Location)` pointing at `decl_path:field_range`.
///
/// Per spec §"LSP support for records": *"Goto-definition on a record literal's
/// field name resolves to the declared field's source span in the corresponding
/// record type."*
///
/// Pure — callers supply the pre-resolved declaration path and field range.
pub fn goto_def_for_record_literal_field(
    decl_path: &std::path::Path,
    field_range: Range,
) -> Option<Location> {
    let uri = Url::from_file_path(decl_path).ok()?;
    Some(Location {
        uri,
        range: field_range,
    })
}

/// Goto-definition on a loader's `path` argument — resolves to the loaded file
/// at the beginning of the file.
///
/// Returns `Some(Location { uri: file://{ws_root/rel_path}, range: line 0 col 0 })`.
///
/// Per spec §"LSP support (loader subset)": *"Goto-definition on the `path`
/// argument resolves to the loaded file (cursor on row 1)."* The spec speaks
/// in user-visible 1-indexed rows; the LSP protocol encodes positions
/// 0-indexed, so user-visible row 1 is `Position { line: 0, character: 0 }`.
///
/// Pure — callers supply the workspace root and the relative path string.
pub fn goto_def_for_loader_path(
    workspace_root: &std::path::Path,
    rel_path: &str,
) -> Option<Location> {
    let full_path = workspace_root.join(rel_path);
    let uri = Url::from_file_path(&full_path).ok()?;
    Some(Location {
        uri,
        range: Range {
            start: Position::new(0, 0),
            end: Position::new(0, 0),
        },
    })
}

/// Goto-definition on a record-typed field of a loaded value, projected at the
/// consumer site — resolves to the YAML/JSON row that produced the value.
///
/// Returns `Some(Location)` anchored at `yaml_file` line `row`, column 0.
///
/// Per spec §"LSP support (loader subset)": *"Goto-definition on a
/// record-typed field of a loaded value, projected at the consumer site,
/// resolves to the YAML/JSON row that produced the value (when statically
/// traceable)."*
///
/// Pure — callers supply the YAML/JSON file path and the 0-indexed row number.
pub fn goto_def_for_loaded_record_field_projection(
    yaml_file: &std::path::Path,
    row: u32,
) -> Option<Location> {
    let uri = Url::from_file_path(yaml_file).ok()?;
    Some(Location {
        uri,
        range: Range {
            start: Position::new(row, 0),
            end: Position::new(row, 0),
        },
    })
}

/// Determine whether the text immediately before `cursor_offset` in `sql`
/// ends with `<param_name>.` where `<param_name>` is a lambda parameter
/// that is `ModelRef`-typed — i.e. it is bound by a HOF (`map`, `filter`,
/// or `reduce`) whose **first argument** is a `smelt.models.with_tag(...)` or
/// `smelt.models.all` call.
///
/// Returns `Some(param_name)` when the condition holds, `None` otherwise.
///
/// Pure — no Salsa dependency.
pub fn is_model_ref_param_before_dot(
    file: &smelt_parser::ast::File,
    sql: &str,
    cursor_offset: usize,
) -> Option<String> {
    use smelt_parser::syntax_kind::SyntaxKind;

    let before = &sql[..cursor_offset.min(sql.len())];
    let dot_trimmed = before.strip_suffix('.')?;
    let param_name: String = dot_trimmed
        .chars()
        .rev()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    if param_name.is_empty() {
        return None;
    }

    // Find the innermost LAMBDA that contains cursor_offset and declares param_name.
    let lambda_node = file
        .syntax()
        .descendants()
        .filter(|n| n.kind() == SyntaxKind::LAMBDA)
        .filter(|n| {
            let s: usize = n.text_range().start().into();
            let e: usize = n.text_range().end().into();
            cursor_offset >= s && cursor_offset <= e
        })
        .filter_map(smelt_parser::ast::Lambda::cast)
        .find(|lam| lam.params().iter().any(|p| p == &param_name))?;

    // Walk up to the nearest enclosing HOF FUNCTION_CALL.
    let mut cur = lambda_node.syntax().parent();
    let mut hof_call: Option<smelt_parser::ast::FunctionCall> = None;
    while let Some(node) = cur {
        if node.kind() == SyntaxKind::FUNCTION_CALL {
            if let Some(fc) = smelt_parser::ast::FunctionCall::cast(node.clone()) {
                if matches!(
                    hof_call_name(&fc).as_deref().unwrap_or(""),
                    "map" | "filter" | "reduce"
                ) {
                    hof_call = Some(fc);
                    break;
                }
            }
        }
        cur = node.parent();
    }

    // Check that the HOF's first argument is `smelt.models.with_tag(...)` or
    // `smelt.models.all` (or `smelt.models.all()`).
    let hof = hof_call?;
    let first_arg = hof.arguments().into_iter().next()?;
    let path_call = first_arg.as_smelt_path_call()?;
    let segs = path_call.segments();
    // segs for `smelt.models.with_tag(...)` → ["models", "with_tag"]
    // segs for `smelt.models.all` / `smelt.models.all()` → ["models", "all"]
    if segs.first().map(|s| s.as_str()) == Some("models")
        && segs
            .get(1)
            .map(|s| s.as_str() == "with_tag" || s.as_str() == "all")
            .unwrap_or(false)
    {
        return Some(param_name);
    }
    None
}

/// Determine whether the text immediately before `cursor_offset` in `sql`
/// ends with `<param_name>.` where `<param_name>` is a lambda parameter
/// that is `SourceRef`-typed — i.e. it is bound by a HOF whose **first
/// argument** is a `smelt.sources.with_tag(...)` or `smelt.sources.all` call.
///
/// Returns `Some(param_name)` when the condition holds, `None` otherwise.
///
/// Pure — no Salsa dependency.
pub fn is_source_ref_param_before_dot(
    file: &smelt_parser::ast::File,
    sql: &str,
    cursor_offset: usize,
) -> Option<String> {
    use smelt_parser::syntax_kind::SyntaxKind;

    let before = &sql[..cursor_offset.min(sql.len())];
    let dot_trimmed = before.strip_suffix('.')?;
    let param_name: String = dot_trimmed
        .chars()
        .rev()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    if param_name.is_empty() {
        return None;
    }

    let lambda_node = file
        .syntax()
        .descendants()
        .filter(|n| n.kind() == SyntaxKind::LAMBDA)
        .filter(|n| {
            let s: usize = n.text_range().start().into();
            let e: usize = n.text_range().end().into();
            cursor_offset >= s && cursor_offset <= e
        })
        .filter_map(smelt_parser::ast::Lambda::cast)
        .find(|lam| lam.params().iter().any(|p| p == &param_name))?;

    let mut cur = lambda_node.syntax().parent();
    let mut hof_call: Option<smelt_parser::ast::FunctionCall> = None;
    while let Some(node) = cur {
        if node.kind() == SyntaxKind::FUNCTION_CALL {
            if let Some(fc) = smelt_parser::ast::FunctionCall::cast(node.clone()) {
                if matches!(
                    hof_call_name(&fc).as_deref().unwrap_or(""),
                    "map" | "filter" | "reduce"
                ) {
                    hof_call = Some(fc);
                    break;
                }
            }
        }
        cur = node.parent();
    }

    let hof = hof_call?;
    let first_arg = hof.arguments().into_iter().next()?;
    let path_call = first_arg.as_smelt_path_call()?;
    let segs = path_call.segments();
    if segs.first().map(|s| s.as_str()) == Some("sources")
        && segs
            .get(1)
            .map(|s| s.as_str() == "with_tag" || s.as_str() == "all")
            .unwrap_or(false)
    {
        return Some(param_name);
    }
    None
}
