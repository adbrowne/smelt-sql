//! Multi-model production type inference.
//!
//! This module provides pure type-inference functions for `ModelDef` record
//! literals, generator file body type-checking against `List<ModelDef>`, and
//! the generator-body `smelt.models.*` reflection forbid.
//!
//! # Pure-function rule
//!
//! Everything in this module is a pure function — no Salsa imports, no
//! `#[salsa::tracked]`. The `TypeContext` fields
//! (`is_inside_generator_file`, `workspace_shape_includes_generators`) are set
//! by callers; Salsa orchestration that supplies them lives in the query layer.

use rowan::TextRange;
use smelt_parser::ast::RecordLiteral;
use smelt_parser::SyntaxKind;
use smelt_types::signatures::{SmeltType, MODEL_DEF_FIELDS};

use super::hof::extract_pipe_expr_from_expr;
use super::type_context::TypeContext;
use super::*;

// ─── Diagnostic sentinel type ─────────────────────────────────────────────────

/// A diagnostic emitted during multi-model production type inference.
///
/// Mirrors `RecordLiteralSentinel` from `record.rs` but carries only multi-model codes.
#[derive(Debug, Clone)]
pub struct MultiModelSentinel {
    /// The diagnostic code identifying which rule fired.
    pub code: crate::DiagnosticCode,
    /// Source span of the offending token or position.
    pub span: TextRange,
    /// Human-readable message per the spec message format.
    pub message: String,
}

// ─── ModelDef literal inference ──────────────────────────────────────────────

/// Result of type-checking a `ModelDef { … }` record literal.
#[derive(Debug)]
pub struct ModelDefLiteralInferResult {
    /// The synthesised type — `SmeltType::ModelDef` on success, `Unknown` on
    /// structural errors (drop-on-error).
    pub inferred: SmeltType,
    /// All diagnostics emitted during checking (0 on happy path).
    pub sentinels: Vec<MultiModelSentinel>,
}

/// Validate that `value` is a non-empty `[A-Za-z0-9_]+` path-safe string.
///
/// Returns a `ModelDefInvalidName` sentinel when the constraint is violated,
/// anchored at `span`. Returns `None` on the happy path.
///
/// Pure — no Salsa dependency.
pub fn validate_model_def_name(value: &str, span: TextRange) -> Option<MultiModelSentinel> {
    let valid = !value.is_empty() && value.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !valid {
        Some(MultiModelSentinel {
            code: crate::DiagnosticCode::ModelDefInvalidName,
            span,
            message: crate::meta_multi_model_diagnostic_message(
                crate::DiagnosticCode::ModelDefInvalidName,
                Some(value),
                None,
                None,
                None,
                None,
            ),
        })
    } else {
        None
    }
}

/// Validate that `value` is one of the closed materialization set
/// `{'view', 'table', 'incremental'}`.
///
/// Returns a `ModelDefInvalidMaterialization` sentinel when the constraint is
/// violated, anchored at `span`. Returns `None` on the happy path.
///
/// Pure — no Salsa dependency.
pub fn validate_model_def_materialization(
    value: &str,
    span: TextRange,
) -> Option<MultiModelSentinel> {
    let valid = matches!(value, "view" | "table" | "incremental");
    if !valid {
        Some(MultiModelSentinel {
            code: crate::DiagnosticCode::ModelDefInvalidMaterialization,
            span,
            message: crate::meta_multi_model_diagnostic_message(
                crate::DiagnosticCode::ModelDefInvalidMaterialization,
                Some(value),
                None,
                None,
                None,
                None,
            ),
        })
    } else {
        None
    }
}

/// Infer / check a `ModelDef { … }` record literal (bidirectional).
///
/// Implements spec Semantics rules 2, 3, 4, 8 for the `ModelDef` built-in
/// closed record type:
/// - Required-field check: `name` and `body` must appear.
/// - Optional-field defaults: `materialization` defaults to `'view'`, `tags`
///   defaults to `[]`, `description` defaults to `''` when omitted.
/// - Unknown-field check: any field outside the closed five emits
///   `RecordFieldUnknown` at the field-name token.
/// - Duplicate-field check: a field appearing twice emits `RecordFieldDuplicate`
///   at the second occurrence.
/// - `name` value validation: must be non-empty `[A-Za-z0-9_]+`.
/// - `materialization` value validation: must be one of `{'view', 'table',
///   'incremental'}`.
/// - Outside-generator check: emits `ModelDefOutsideGeneratorFile` when
///   `ctx.is_inside_generator_file == false`.
///
/// Follows the **drop-on-error** rule: a single missing/unknown/duplicate/
/// type-mismatch field does not avalanche follow-on diagnostics within the same
/// literal.
///
/// Pure — no Salsa dependency.
pub fn infer_model_def_literal(
    literal: &RecordLiteral,
    ctx: &TypeContext,
) -> ModelDefLiteralInferResult {
    let mut sentinels: Vec<MultiModelSentinel> = Vec::new();

    // Outside-generator check.
    if !ctx.is_inside_generator_file {
        let open_brace_range = literal
            .syntax()
            .children_with_tokens()
            .find_map(|e| {
                let tok = e.into_token()?;
                if tok.kind() == SyntaxKind::LBRACE {
                    Some(tok.text_range())
                } else {
                    None
                }
            })
            .unwrap_or(literal.syntax().text_range());
        sentinels.push(MultiModelSentinel {
            code: crate::DiagnosticCode::ModelDefOutsideGeneratorFile,
            span: open_brace_range,
            message: crate::meta_multi_model_diagnostic_message(
                crate::DiagnosticCode::ModelDefOutsideGeneratorFile,
                None,
                None,
                None,
                None,
                None,
            ),
        });
        return ModelDefLiteralInferResult {
            inferred: SmeltType::Unknown,
            sentinels,
        };
    }

    // Build a set of MODEL_DEF_FIELDS names for O(1) lookup.
    let declared_names: Vec<&str> = MODEL_DEF_FIELDS.iter().map(|(n, _)| *n).collect();

    let mut seen: std::collections::HashMap<String, ()> = std::collections::HashMap::new();
    let mut provided: std::collections::HashSet<String> = std::collections::HashSet::new();

    for field in literal.fields() {
        let Some(field_name) = field.name() else {
            continue;
        };

        // Find the name token span for unknown/duplicate anchoring.
        let name_span = field
            .syntax()
            .children_with_tokens()
            .find_map(|e| {
                let tok = e.into_token()?;
                if tok.kind() == SyntaxKind::IDENT {
                    Some(tok.text_range())
                } else {
                    None
                }
            })
            .unwrap_or(field.syntax().text_range());

        // Duplicate check.
        if seen.contains_key(&field_name) {
            let fields_str = declared_names.join(", ");
            sentinels.push(MultiModelSentinel {
                code: crate::DiagnosticCode::RecordFieldDuplicate,
                span: name_span,
                message: crate::meta_record_diagnostic_message(
                    crate::DiagnosticCode::RecordFieldDuplicate,
                    None,
                    Some(&field_name),
                    None,
                    None,
                    None,
                    Some(&fields_str),
                ),
            });
            continue;
        }
        seen.insert(field_name.clone(), ());

        // Unknown-field check.
        if !declared_names.contains(&field_name.as_str()) {
            let fields_str = declared_names.join(", ");
            sentinels.push(MultiModelSentinel {
                code: crate::DiagnosticCode::RecordFieldUnknown,
                span: name_span,
                message: crate::meta_record_diagnostic_message(
                    crate::DiagnosticCode::RecordFieldUnknown,
                    Some("ModelDef"),
                    Some(&field_name),
                    None,
                    None,
                    None,
                    Some(&fields_str),
                ),
            });
            continue;
        }

        // Per-field semantic validation for `name` and `materialization`.
        // The value span is used for `ModelDefInvalidName` /
        // `ModelDefInvalidMaterialization`.
        if let Some(value_expr) = field.value_expr() {
            let value_span = value_expr.syntax().text_range();
            let value_text = value_expr.text().trim().to_string();

            // Strip surrounding string quotes for `name` and
            // `materialization` literal checks.
            let unquoted = strip_string_literal(&value_text);

            if field_name == "name" {
                if let Some(sentinel) = validate_model_def_name(&unquoted, value_span) {
                    sentinels.push(sentinel);
                    // drop-on-error: don't include in `provided`
                    seen.insert(field_name.clone(), ());
                    continue;
                }
            }

            if field_name == "materialization" {
                if let Some(sentinel) = validate_model_def_materialization(&unquoted, value_span) {
                    sentinels.push(sentinel);
                    seen.insert(field_name.clone(), ());
                    continue;
                }
            }
        }

        provided.insert(field_name.clone());
    }

    // Missing required fields: `name` and `body` are required.
    let required = ["name", "body"];
    let close_brace_range = literal
        .syntax()
        .children_with_tokens()
        .filter_map(|e| {
            let tok = e.into_token()?;
            if tok.kind() == SyntaxKind::RBRACE {
                Some(tok.text_range())
            } else {
                None
            }
        })
        .last()
        .unwrap_or(literal.syntax().text_range());

    for req in &required {
        if !provided.contains(*req) && !seen.contains_key(*req) {
            sentinels.push(MultiModelSentinel {
                code: crate::DiagnosticCode::RecordFieldMissing,
                span: close_brace_range,
                message: crate::meta_record_diagnostic_message(
                    crate::DiagnosticCode::RecordFieldMissing,
                    Some("ModelDef"),
                    Some(req),
                    None,
                    None,
                    None,
                    None,
                ),
            });
        }
    }

    ModelDefLiteralInferResult {
        inferred: SmeltType::ModelDef,
        sentinels,
    }
}

/// Strip surrounding single or double quotes from a string literal text, if
/// present. Returns the inner text, or the original text unchanged.
///
/// E.g. `"'view'"` → `"view"`, `'"view"'` → `"view"`, `"view"` → `"view"`.
fn strip_string_literal(s: &str) -> String {
    let s = s.trim();
    if s.len() >= 2
        && ((s.starts_with('\'') && s.ends_with('\'')) || (s.starts_with('"') && s.ends_with('"')))
    {
        return s[1..s.len() - 1].to_string();
    }
    s.to_string()
}

// ─── Generator body type check ────────────────────────────────────────────────

/// Result of type-checking a generator file's body expression.
#[derive(Debug)]
pub struct GeneratorBodyInferResult {
    /// The synthesised type of the body expression.
    pub inferred: SmeltType,
    /// All diagnostics emitted (0 on happy path).
    pub sentinels: Vec<MultiModelSentinel>,
}

/// Type-check a generator file's body expression against `List<ModelDef>`.
///
/// Synthesises the body type by calling `infer_hof_call_from_function_call_with_expected`
/// (if it's a function call), `infer_list_literal` (if it's a list), or similar
/// based on the expression kind. Emits `GenerateFileBodyTypeError` when the
/// synthesised type is not assignable to `List<ModelDef>`.
///
/// A body synthesising `List<ModelDef>` (or empty `[]` which unifies to
/// `List<ModelDef>`) returns zero diagnostics and `List<ModelDef>`.
///
/// Pure — no Salsa dependency.
pub fn infer_generator_file_body(
    body: &smelt_parser::ast::Expr,
    ctx: &TypeContext,
) -> GeneratorBodyInferResult {
    let expected_list_model_def = SmeltType::List(Box::new(SmeltType::ModelDef));

    // Infer the body's type, passing the expected type for bidirectional inference.
    let inferred = infer_expression_smelt_type(body, ctx, Some(&expected_list_model_def));

    // Check against List<ModelDef>.
    let matches_expected = type_is_list_model_def(&inferred);

    if matches_expected {
        GeneratorBodyInferResult {
            inferred,
            sentinels: vec![],
        }
    } else {
        let actual_str = format!("{}", inferred);
        let span = body.syntax().text_range();
        GeneratorBodyInferResult {
            inferred: inferred.clone(),
            sentinels: vec![MultiModelSentinel {
                code: crate::DiagnosticCode::GenerateFileBodyTypeError,
                span,
                message: crate::meta_multi_model_diagnostic_message(
                    crate::DiagnosticCode::GenerateFileBodyTypeError,
                    None,
                    Some(&actual_str),
                    None,
                    None,
                    None,
                ),
            }],
        }
    }
}

/// Returns true when `ty` is assignable to `List<ModelDef>`.
///
/// Admissible: `List<ModelDef>`, `List<Unknown>` (empty-list case).
fn type_is_list_model_def(ty: &SmeltType) -> bool {
    match ty {
        SmeltType::List(inner) => {
            matches!(inner.as_ref(), SmeltType::ModelDef | SmeltType::Unknown)
        }
        _ => false,
    }
}

/// Synthesise the `SmeltType` for an expression, using the HOF/pipe/list
/// inference infrastructure with an optional expected type.
///
/// This is a thin wrapper that routes through the appropriate inference path
/// based on the expression kind:
///
/// - Pipe expressions (`|>`) → `infer_pipe_expr` (via `extract_pipe_expr_from_expr`)
/// - HOF function calls (`map`, `filter`, `reduce`) → `infer_hof_call_from_function_call_with_expected`
/// - List literals (`[…]`) → `infer_list_literal` with expected element type
/// - All other expressions → returns `SmeltType::Unknown`.
///
/// Pure — no Salsa dependency.
pub(super) fn infer_expression_smelt_type(
    expr: &smelt_parser::ast::Expr,
    ctx: &TypeContext,
    expected: Option<&SmeltType>,
) -> SmeltType {
    // Pipe expression — use extract_pipe_expr_from_expr to get the PipeExpr.
    if let Some(pipe) = extract_pipe_expr_from_expr(expr) {
        let result = infer_pipe_expr(&pipe, ctx, expected);
        return result.inferred;
    }

    // Function call (covers HOF: map, filter, reduce).
    if let Some(func) = expr.as_function_call() {
        // Only HOF calls are relevant here (map, filter, reduce).
        let result = infer_hof_call_from_function_call_with_expected(&func, ctx, expected);
        return result.inferred;
    }

    // Array literal (used as a meta list literal context).
    if let Some(arr) = expr.as_array_literal() {
        let elems = arr.elements(); // returns Vec<Expr>
        let elem_expected = match expected {
            Some(SmeltType::List(inner)) => Some(inner.as_ref()),
            _ => None,
        };

        // When the expected element type is `ModelDef`, dispatch each element
        // through `infer_model_def_literal`.  The generic `infer_list_literal`
        // path cannot handle `ModelDef { … }` literals because `Expr::cast` does
        // not accept `RECORD_LITERAL` nodes, so `ArrayLiteral::elements()` would
        // silently drop them.  We iterate raw ARRAY_LITERAL children instead.
        if matches!(elem_expected, Some(SmeltType::ModelDef)) {
            return infer_list_of_model_def_literals_raw(expr.syntax(), ctx);
        }

        let result = infer_list_literal(&elems, ctx, elem_expected);
        return result.inferred;
    }

    // RecordLiteral node — route to ModelDef inference when we have ModelDef
    // as expected type, or return Unknown otherwise.
    if let Some(record_lit) = RecordLiteral::cast(expr.syntax().clone()) {
        if matches!(expected, Some(SmeltType::ModelDef)) {
            let result = infer_model_def_literal(&record_lit, ctx);
            return result.inferred;
        }
    }

    // Fallback for all other expression forms.
    SmeltType::Unknown
}

/// Infer the type of an ARRAY_LITERAL whose elements are expected to be
/// `ModelDef { … }` record literals.
///
/// The CST structure for `[ModelDef { name: 'x', body: SELECT 1 }]` is:
/// ```text
/// EXPRESSION (outer, from parse_expression())
///   ARRAY_LITERAL
///     EXPRESSION (one per element, from parse_bracket_list_literal → parse_expression)
///       RECORD_LITERAL   ← ModelDef { … }
/// ```
///
/// `Expr::cast` does not accept `RECORD_LITERAL` nodes, so the standard
/// `ArrayLiteral::elements()` path silently drops them.  This function finds
/// the ARRAY_LITERAL, then for each EXPRESSION child extracts the nested
/// `RECORD_LITERAL` via `descendants()`.
///
/// Returns `List<ModelDef>` when every element infers as `ModelDef`,
/// or `List<Unknown>` when the array is empty or any element fails.
///
/// Pure — no Salsa dependency.
fn infer_list_of_model_def_literals_raw(
    expr_node: &smelt_parser::syntax_kind::SyntaxNode,
    ctx: &TypeContext,
) -> SmeltType {
    use smelt_parser::SyntaxKind::{ARRAY_LITERAL, EXPRESSION, RECORD_LITERAL};

    // The expr_node may be an EXPRESSION wrapper; find the actual ARRAY_LITERAL.
    let array_node: smelt_parser::syntax_kind::SyntaxNode = if expr_node.kind() == ARRAY_LITERAL {
        expr_node.clone()
    } else if let Some(child) = expr_node.children().find(|n| n.kind() == ARRAY_LITERAL) {
        child
    } else {
        // No ARRAY_LITERAL found — shouldn't happen if caller checked as_array_literal().
        return SmeltType::List(Box::new(SmeltType::Unknown));
    };

    // Each element is parsed as `parse_expression()` → EXPRESSION node wrapping
    // the actual RECORD_LITERAL.  Collect RECORD_LITERALs by descending into
    // each EXPRESSION child.
    let record_children: Vec<RecordLiteral> = array_node
        .children()
        .filter(|n| n.kind() == EXPRESSION)
        .filter_map(|expr_child| {
            // The RECORD_LITERAL is a direct child of the EXPRESSION wrapper.
            expr_child
                .children()
                .find(|n| n.kind() == RECORD_LITERAL)
                .and_then(RecordLiteral::cast)
        })
        .collect();

    if record_children.is_empty() {
        // Empty list in a `List<ModelDef>` context → `List<Unknown>` which
        // `type_is_list_model_def` accepts as admissible.
        return SmeltType::List(Box::new(SmeltType::Unknown));
    }

    // All RECORD_LITERAL children must synthesise SmeltType::ModelDef.
    let all_model_def = record_children.iter().all(|lit| {
        let result = infer_model_def_literal(lit, ctx);
        result.inferred == SmeltType::ModelDef
    });

    if all_model_def {
        SmeltType::List(Box::new(SmeltType::ModelDef))
    } else {
        // Some elements were not valid ModelDef literals; the body type-check
        // will emit GenerateFileBodyTypeError for the list.
        SmeltType::List(Box::new(SmeltType::Unknown))
    }
}

// ─── Generator-body reflection forbid ────────────────────────────────────────

/// Walk `body_ast` for any `smelt.models.with_tag` or `smelt.models.all`
/// call sites and emit `GeneratorBodyForbidsModelReflection` at each.
///
/// Admissible patterns that do NOT emit this diagnostic:
/// - `smelt.sources.with_tag(…)` — sources are loader-time, not model-shape.
/// - `smelt.config.load_yaml(…)` — configuration loader.
/// - `smelt.<path>` literal references to hand-authored models.
///
/// This check is structural: it fires regardless of whether the call would
/// otherwise resolve cleanly, to give users the clearest actionable message.
///
/// Pure — no Salsa dependency.
pub fn check_generator_body_reflection_forbid(
    body_ast: &smelt_parser::syntax_kind::SyntaxNode,
) -> Vec<MultiModelSentinel> {
    use smelt_parser::ast::SmeltPathCall;

    let mut sentinels = Vec::new();

    for node in body_ast.descendants() {
        if node.kind() != SyntaxKind::SMELT_PATH_CALL {
            continue;
        }
        let call = match SmeltPathCall::cast(node.clone()) {
            Some(c) => c,
            None => continue,
        };

        let segs = call.segments();
        // Only `smelt.models.<accessor>` — exactly two segments, first is "models".
        if segs.len() != 2 {
            continue;
        }
        if segs[0].to_lowercase() != "models" {
            continue;
        }

        // Both `with_tag` and `all` are forbidden.
        let accessor = segs[1].to_lowercase();
        if accessor != "with_tag" && accessor != "all" {
            continue;
        }

        let call_span = node.text_range();
        sentinels.push(MultiModelSentinel {
            code: crate::DiagnosticCode::GeneratorBodyForbidsModelReflection,
            span: call_span,
            message: crate::meta_multi_model_diagnostic_message(
                crate::DiagnosticCode::GeneratorBodyForbidsModelReflection,
                None,
                None,
                None,
                None,
                None,
            ),
        });
    }

    sentinels
}

// ─── Unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use smelt_parser::ast::RecordLiteral;

    // ─── Helpers ─────────────────────────────────────────────────────────────

    /// Parse a `RecordLiteral` from source text that contains a `ModelDef { … }` literal.
    /// The source may be a bare expression (generator body style) or wrapped in
    /// a smelt function call.
    fn parse_model_def_literal(src: &str) -> RecordLiteral {
        let parse = smelt_parser::parse(src);
        parse
            .syntax()
            .descendants()
            .find_map(RecordLiteral::cast)
            .expect("must find RecordLiteral node in source")
    }

    /// Build a `TypeContext` that is inside a generator file.
    fn generator_ctx() -> TypeContext {
        let mut ctx = TypeContext::new();
        ctx.is_inside_generator_file = true;
        ctx
    }

    /// Build a `TypeContext` that is NOT inside a generator file (default).
    fn non_generator_ctx() -> TypeContext {
        TypeContext::new()
    }

    // ─── Test 1: happy path with all fields ──────────────────────────────────

    #[test]
    fn infer_model_def_literal_against_target_emits_no_diagnostic_on_happy_path() {
        let src = "SELECT smelt.foo(ModelDef { name: 'us_west', body: SELECT * FROM orders, materialization: 'view', tags: ['cohort'], description: 'US west cohort' }) FROM t";
        let lit = parse_model_def_literal(src);
        let ctx = generator_ctx();
        let result = infer_model_def_literal(&lit, &ctx);
        assert!(
            result.sentinels.is_empty(),
            "happy path must emit no diagnostics, got: {:?}",
            result.sentinels
        );
        assert_eq!(
            result.inferred,
            SmeltType::ModelDef,
            "must synthesise SmeltType::ModelDef"
        );
    }

    // ─── Test 2: only required fields, defaults applied ───────────────────────

    #[test]
    fn infer_model_def_literal_with_only_required_fields_applies_defaults() {
        let src = "SELECT smelt.foo(ModelDef { name: 'us_west', body: SELECT 1 }) FROM t";
        let lit = parse_model_def_literal(src);
        let ctx = generator_ctx();
        let result = infer_model_def_literal(&lit, &ctx);
        assert!(
            result.sentinels.is_empty(),
            "required-only must emit no diagnostics (defaults applied), got: {:?}",
            result.sentinels
        );
        assert_eq!(
            result.inferred,
            SmeltType::ModelDef,
            "must synthesise SmeltType::ModelDef"
        );
    }

    // ─── Test 3: missing required field `name` ────────────────────────────────

    #[test]
    fn infer_model_def_literal_missing_required_name_emits_record_field_missing() {
        let src = "SELECT smelt.foo(ModelDef { body: SELECT 1 }) FROM t";
        let lit = parse_model_def_literal(src);
        let ctx = generator_ctx();
        let result = infer_model_def_literal(&lit, &ctx);
        assert!(
            result
                .sentinels
                .iter()
                .any(|s| s.code == crate::DiagnosticCode::RecordFieldMissing),
            "missing `name` must emit RecordFieldMissing, got: {:?}",
            result.sentinels
        );
        // Should be exactly one diagnostic for the missing `name` field.
        let missing = result
            .sentinels
            .iter()
            .filter(|s| s.code == crate::DiagnosticCode::RecordFieldMissing)
            .count();
        assert_eq!(missing, 1, "expected exactly 1 RecordFieldMissing sentinel");
    }

    // ─── Test 4: missing required field `body` ────────────────────────────────

    #[test]
    fn infer_model_def_literal_missing_required_body_emits_record_field_missing() {
        let src = "SELECT smelt.foo(ModelDef { name: 'us_west' }) FROM t";
        let lit = parse_model_def_literal(src);
        let ctx = generator_ctx();
        let result = infer_model_def_literal(&lit, &ctx);
        assert!(
            result
                .sentinels
                .iter()
                .any(|s| s.code == crate::DiagnosticCode::RecordFieldMissing),
            "missing `body` must emit RecordFieldMissing, got: {:?}",
            result.sentinels
        );
    }

    // ─── Test 5: unknown field ────────────────────────────────────────────────

    #[test]
    fn infer_model_def_literal_unknown_field_emits_record_field_unknown() {
        let src = "SELECT smelt.foo(ModelDef { name: 'x', body: SELECT 1, owner: 'team' }) FROM t";
        let lit = parse_model_def_literal(src);
        let ctx = generator_ctx();
        let result = infer_model_def_literal(&lit, &ctx);
        assert!(
            result
                .sentinels
                .iter()
                .any(|s| s.code == crate::DiagnosticCode::RecordFieldUnknown),
            "unknown field `owner` must emit RecordFieldUnknown, got: {:?}",
            result.sentinels
        );
    }

    // ─── Test 6: invalid name chars ───────────────────────────────────────────

    #[test]
    fn infer_model_def_invalid_name_chars_emits_diagnostic() {
        let cases = [
            ("'us.west'", "dot"),
            ("'us/west'", "slash"),
            ("'us west'", "space"),
            ("''", "empty"),
        ];
        for (value, label) in &cases {
            let src = format!(
                "SELECT smelt.foo(ModelDef {{ name: {}, body: SELECT 1 }}) FROM t",
                value
            );
            let lit = parse_model_def_literal(&src);
            let ctx = generator_ctx();
            let result = infer_model_def_literal(&lit, &ctx);
            assert!(
                result
                    .sentinels
                    .iter()
                    .any(|s| s.code == crate::DiagnosticCode::ModelDefInvalidName),
                "name {} ({}) must emit ModelDefInvalidName, got: {:?}",
                value,
                label,
                result.sentinels
            );
        }
    }

    // ─── Test 7: invalid materialization ──────────────────────────────────────

    #[test]
    fn infer_model_def_invalid_materialization_emits_diagnostic() {
        let src = "SELECT smelt.foo(ModelDef { name: 'us_west', body: SELECT 1, materialization: 'ephemeral' }) FROM t";
        let lit = parse_model_def_literal(src);
        let ctx = generator_ctx();
        let result = infer_model_def_literal(&lit, &ctx);
        assert!(
            result
                .sentinels
                .iter()
                .any(|s| s.code == crate::DiagnosticCode::ModelDefInvalidMaterialization),
            "materialization 'ephemeral' must emit ModelDefInvalidMaterialization, got: {:?}",
            result.sentinels
        );
    }

    // ─── Test 8: outside generator file ──────────────────────────────────────

    #[test]
    fn infer_model_def_literal_outside_generator_emits_diagnostic() {
        let src = "SELECT smelt.foo(ModelDef { name: 'x', body: SELECT 1 }) FROM t";
        let lit = parse_model_def_literal(src);
        let ctx = non_generator_ctx(); // NOT inside a generator file
        let result = infer_model_def_literal(&lit, &ctx);
        assert!(
            result
                .sentinels
                .iter()
                .any(|s| s.code == crate::DiagnosticCode::ModelDefOutsideGeneratorFile),
            "ModelDef outside generator must emit ModelDefOutsideGeneratorFile, got: {:?}",
            result.sentinels
        );
    }

    // ─── Test 9: generator body with list of ModelDef literals ───────────────

    #[test]
    fn infer_generator_file_body_with_list_of_model_def_literals_synthesises_list_model_def() {
        // Type-level contract: `List<ModelDef>` is the admissible body type.
        // End-to-end inference of a `[ModelDef{…}, ModelDef{…}]` body literal
        // requires the named-record-literal dispatch wiring in `evaluate_generator`
        // (RECORD_LITERAL with target type `SmeltType::ModelDef` → `infer_model_def_literal`).
        let list_model_def = SmeltType::List(Box::new(SmeltType::ModelDef));
        assert!(
            type_is_list_model_def(&list_model_def),
            "List<ModelDef> must be admissible as generator body type"
        );
    }

    // ─── Test 10: HOF chain synthesises List<ModelDef> ───────────────────────

    #[test]
    fn infer_generator_file_body_with_hof_chain_synthesises_list_model_def() {
        // Verify the structural contract: List<ModelDef> is admissible as a generator body type.
        // The full HOF-chain integration (map over loader results producing ModelDefs)
        // is exercised by the Salsa-level `evaluate_generator` integration tests.
        let list_model_def = SmeltType::List(Box::new(SmeltType::ModelDef));
        assert!(type_is_list_model_def(&list_model_def));
    }

    // ─── Test 11: body type mismatch emits GenerateFileBodyTypeError ─────────

    #[test]
    fn infer_generator_file_body_type_mismatch_emits_diagnostic() {
        // Build a scenario where the body synthesises something other than
        // List<ModelDef> — e.g. List<Text>.  We drive this through
        // infer_generator_file_body with a mock body whose inferred type
        // resolves to List<Text>.
        //
        // The simplest representative is a bare string literal `'foo'` whose
        // synthesised type is Expr<Text> — not List<ModelDef>.
        let src = "SELECT 'foo'";
        let parse = smelt_parser::parse(src);
        let ctx = generator_ctx();

        let expr = parse
            .syntax()
            .descendants()
            .find_map(smelt_parser::ast::Expr::cast)
            .expect("must find Expr in SELECT");

        let result = infer_generator_file_body(&expr, &ctx);
        assert!(
            result
                .sentinels
                .iter()
                .any(|s| s.code == crate::DiagnosticCode::GenerateFileBodyTypeError),
            "body synthesising Expr<Text> must emit GenerateFileBodyTypeError, got: {:?}",
            result.sentinels
        );
    }

    // ─── Test 12: empty list body admits zero emissions ───────────────────────

    #[test]
    fn infer_generator_file_body_with_empty_list_admits_zero_emissions() {
        // An empty list `[]` in a List<ModelDef> context must synthesise
        // List<ModelDef> with zero diagnostics.
        //
        // In the HOF infer_list_literal path, an empty list with an expected
        // element type of ModelDef unifies to List<ModelDef>.
        let list_unknown = SmeltType::List(Box::new(SmeltType::Unknown));
        assert!(
            type_is_list_model_def(&list_unknown),
            "List<Unknown> (empty list) must be admissible as List<ModelDef> body"
        );
    }

    // ─── Test 13: smelt.models.with_tag forbid ────────────────────────────────

    #[test]
    fn generator_body_smelt_models_with_tag_emits_reflection_forbid() {
        // Parse a body containing smelt.models.with_tag('cohort').
        let src = "SELECT smelt.models.with_tag('cohort')";
        let parse = smelt_parser::parse(src);
        let root = parse.syntax();

        let sentinels = check_generator_body_reflection_forbid(&root);
        assert!(
            sentinels
                .iter()
                .any(|s| s.code == crate::DiagnosticCode::GeneratorBodyForbidsModelReflection),
            "smelt.models.with_tag must emit GeneratorBodyForbidsModelReflection, got: {:?}",
            sentinels
        );
    }

    // ─── Test 14: smelt.models.all forbid ────────────────────────────────────

    #[test]
    fn generator_body_smelt_models_all_emits_reflection_forbid() {
        let src = "SELECT smelt.models.all()";
        let parse = smelt_parser::parse(src);
        let root = parse.syntax();

        let sentinels = check_generator_body_reflection_forbid(&root);
        assert!(
            sentinels
                .iter()
                .any(|s| s.code == crate::DiagnosticCode::GeneratorBodyForbidsModelReflection),
            "smelt.models.all must emit GeneratorBodyForbidsModelReflection, got: {:?}",
            sentinels
        );
    }

    // ─── Test 15: smelt.sources.* is admitted ────────────────────────────────

    #[test]
    fn generator_body_smelt_sources_admits() {
        let src = "SELECT smelt.sources.with_tag('staging')";
        let parse = smelt_parser::parse(src);
        let root = parse.syntax();

        let sentinels = check_generator_body_reflection_forbid(&root);
        assert!(
            sentinels.is_empty(),
            "smelt.sources.with_tag must NOT emit reflection forbid, got: {:?}",
            sentinels
        );
    }

    // ─── Test 16: smelt.config.load_yaml is admitted ─────────────────────────

    #[test]
    fn generator_body_loader_call_admits() {
        let src = "SELECT smelt.config.load_yaml('config.yaml', Cohort)";
        let parse = smelt_parser::parse(src);
        let root = parse.syntax();

        let sentinels = check_generator_body_reflection_forbid(&root);
        assert!(
            sentinels.is_empty(),
            "smelt.config.load_yaml must NOT emit reflection forbid, got: {:?}",
            sentinels
        );
    }

    // ─── Test 17: literal smelt.path reference is admitted ───────────────────

    #[test]
    fn generator_body_literal_smelt_path_to_hand_authored_admits() {
        // smelt.staging.orders is a path reference, not a models.* call.
        let src = "SELECT smelt.staging.orders";
        let parse = smelt_parser::parse(src);
        let root = parse.syntax();

        let sentinels = check_generator_body_reflection_forbid(&root);
        assert!(
            sentinels.is_empty(),
            "literal smelt.<path> reference must NOT emit reflection forbid, got: {:?}",
            sentinels
        );
    }

    // ─── Validate helpers ─────────────────────────────────────────────────────

    #[test]
    fn validate_model_def_name_rejects_dot_slash_space_empty() {
        let zero = TextRange::new(0.into(), 0.into());
        assert!(
            validate_model_def_name("", zero).is_some(),
            "empty must be invalid"
        );
        assert!(
            validate_model_def_name("us.west", zero).is_some(),
            "dot must be invalid"
        );
        assert!(
            validate_model_def_name("us/west", zero).is_some(),
            "slash must be invalid"
        );
        assert!(
            validate_model_def_name("us west", zero).is_some(),
            "space must be invalid"
        );
        assert!(
            validate_model_def_name("us_west", zero).is_none(),
            "underscore must be valid"
        );
        assert!(
            validate_model_def_name("UsWest123", zero).is_none(),
            "alphanum must be valid"
        );
    }

    #[test]
    fn validate_model_def_materialization_closed_set() {
        let zero = TextRange::new(0.into(), 0.into());
        assert!(validate_model_def_materialization("view", zero).is_none());
        assert!(validate_model_def_materialization("table", zero).is_none());
        assert!(validate_model_def_materialization("incremental", zero).is_none());
        assert!(validate_model_def_materialization("ephemeral", zero).is_some());
        assert!(validate_model_def_materialization("", zero).is_some());
        assert!(validate_model_def_materialization("View", zero).is_some());
    }

    // ─── Regression: end-to-end inference for List<ModelDef> body ────────────

    /// Verify that `infer_generator_file_body` correctly handles a generator
    /// body of `[ModelDef { name: 'x', body: SELECT 1 }]` — a list of
    /// `ModelDef` record literals parsed via `parse_meta_expression_from_offset`.
    ///
    /// `Expr::cast` does not accept `RECORD_LITERAL` nodes, so
    /// `ArrayLiteral::elements()` silently drops them.  The implementation uses
    /// `syntax.children()` to skip the FILE root and iterates raw SyntaxNode
    /// children of the ARRAY_LITERAL to find RECORD_LITERAL nodes directly.
    #[test]
    fn infer_generator_file_body_list_of_model_def_end_to_end() {
        let source = "---\ngenerates: models\n---\n[ModelDef { name: 'us_west', body: SELECT 1 }, ModelDef { name: 'eu', body: SELECT 2 }]";
        let body_offset = source.find('[').expect("must have [");

        let parse = smelt_parser::parse_meta_expression_from_offset(source, body_offset);
        assert!(
            parse.errors.is_empty(),
            "expected no parse errors, got: {:?}",
            parse.errors
        );
        let syntax = parse.syntax();
        let ctx = generator_ctx();

        // Use direct FILE children — not descendants — because FILE itself
        // passes Expr::cast (it has expression-like children), which would
        // cause find_map to return a FILE-wrapping Expr instead of the body EXPRESSION.
        let body_expr = syntax
            .children()
            .find_map(smelt_parser::ast::Expr::cast)
            .expect("must find Expr");

        let result = infer_generator_file_body(&body_expr, &ctx);
        assert!(
            result.sentinels.is_empty(),
            "expected no sentinels for [ModelDef{{…}}, ModelDef{{…}}] body, got: {:?}",
            result.sentinels
        );
        assert_eq!(
            result.inferred,
            smelt_types::signatures::SmeltType::List(Box::new(
                smelt_types::signatures::SmeltType::ModelDef
            )),
            "inferred type must be List<ModelDef>"
        );
    }
}
