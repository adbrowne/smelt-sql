//! Pure helpers for lambda-parameter rename.
//!
//! The rename handler in `backend.rs` delegates to these functions once it
//! determines the cursor is on a lambda parameter binder or a body reference
//! to one.  All helpers are pure (no Salsa) and take the parsed CST + raw
//! source text as input.
//!
//! # Behaviour
//!
//! Renaming a lambda parameter updates the binder occurrence and every
//! reference to it inside the lambda body.  Inner lambdas that shadow the
//! parameter are **not** touched — their bodies use the inner binding, not the
//! outer one.
//!
//! Validation rules (per `docs/specs/lsp.md` §Rename):
//! - The new name must be a valid SQL identifier.
//! - The new name must not be a meta-namespace keyword (`if`, `then`, `else`,
//!   `fn`, `let`).
//! - The new name must not shadow an outer binder that is already referenced
//!   inside the lambda body.

use smelt_parser::ast::File as AstFile;
use smelt_parser::{ast::text_range_to_range, SyntaxKind};

// ─────────────────────────── public types ──────────────────────────────────

/// A single byte-range replacement: `(start_byte, end_byte, new_text)`.
pub type ByteEdit = (usize, usize, String);

/// Successful output of [`rename_lambda_param`].
#[derive(Debug)]
pub enum RenameLambdaResult {
    /// One edit per renamed token (binder + every non-shadowed body use).
    Edits(Vec<ByteEdit>),
    /// The cursor was not on a lambda parameter — this rename handler does
    /// not apply.  The caller should try the next handler.
    NotALambdaParam,
}

/// Failure output of [`rename_lambda_param`].
#[derive(Debug, PartialEq, Eq)]
pub enum RenameLambdaError {
    /// `new_name` is not a valid SQL identifier (e.g. starts with a digit).
    InvalidIdentifier(String),
    /// `new_name` is a reserved meta-namespace keyword.
    ReservedKeyword(String),
    /// `new_name` would shadow an outer binder that is already referenced
    /// inside the lambda body.
    ShadowsOuterBinder(String),
}

impl std::fmt::Display for RenameLambdaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RenameLambdaError::InvalidIdentifier(name) => {
                write!(f, "'{}' is not a valid SQL identifier", name)
            }
            RenameLambdaError::ReservedKeyword(name) => {
                write!(f, "'{}' is a reserved meta-namespace keyword", name)
            }
            RenameLambdaError::ShadowsOuterBinder(name) => {
                write!(
                    f,
                    "'{}' would shadow an outer lambda parameter that is already used in the body",
                    name
                )
            }
        }
    }
}

// ──────────────────────── meta-keyword set ──────────────────────────────────

/// Reserved meta-namespace keywords that may not be used as parameter names.
const META_KEYWORDS: &[&str] = &["if", "then", "else", "fn", "let"];

fn is_meta_keyword(name: &str) -> bool {
    META_KEYWORDS.contains(&name)
}

// ─────────────────────── core implementation ────────────────────────────────

/// Find the innermost `LAMBDA` node whose text span contains `cursor_offset`,
/// and which has a parameter named `param_name` as a binder (not shadowed by
/// a nested lambda).
///
/// Returns `(lambda_node, param_name, binder_byte_range, cursor_token_byte_range)`.
/// `cursor_token_byte_range` is the byte range of the specific token the cursor
/// is on (either the binder token or the body-use token), for use by
/// `prepare_rename_lambda_param`.
fn find_lambda_and_binder_for_cursor(
    file: &AstFile,
    cursor_offset: usize,
) -> Option<(
    smelt_parser::ast::Lambda,
    String,
    smelt_parser::TextRange,
    (usize, usize),
)> {
    use smelt_parser::ast::Lambda;

    // Collect all LAMBDA nodes that contain the cursor, ordered from innermost
    // to outermost (smallest span first).
    let mut candidates: Vec<(Lambda, usize)> = file
        .syntax()
        .descendants()
        .filter(|n| n.kind() == SyntaxKind::LAMBDA)
        .filter_map(|n| {
            let start: usize = n.text_range().start().into();
            let end: usize = n.text_range().end().into();
            if cursor_offset >= start && cursor_offset <= end {
                let span = end - start;
                Lambda::cast(n).map(|lam| (lam, span))
            } else {
                None
            }
        })
        .collect();

    // Sort innermost first (smallest span).
    candidates.sort_by_key(|(_, span)| *span);

    for (lambda, _) in candidates {
        // Check each parameter of this lambda.
        for param_name in lambda.params() {
            // Finding 2: use let-else + continue so a malformed LAMBDA_PARAM
            // (missing IDENT token during error-recovery) is skipped without
            // aborting the entire outer `candidates` loop.
            let Some(binder_range) = crate::hover::lambda_param_binder_range(&lambda, &param_name)
            else {
                continue;
            };
            let binder_start: usize = binder_range.start().into();
            let binder_end: usize = binder_range.end().into();

            // Is the cursor on the binder?
            let on_binder = cursor_offset >= binder_start && cursor_offset <= binder_end;
            if on_binder {
                return Some((lambda, param_name, binder_range, (binder_start, binder_end)));
            }

            // Is the cursor on a body-use IDENT with the same name?
            if let Some(body) = lambda.body() {
                let cursor_token_range = body
                    .syntax()
                    .descendants_with_tokens()
                    .filter_map(|e| e.into_token())
                    .filter(|t| t.kind() == SyntaxKind::IDENT && t.text() == param_name.as_str())
                    .find(|t| {
                        let s: usize = t.text_range().start().into();
                        let e: usize = t.text_range().end().into();
                        cursor_offset >= s && cursor_offset <= e
                    })
                    .map(|t| {
                        let s: usize = t.text_range().start().into();
                        let e: usize = t.text_range().end().into();
                        (s, e)
                    });
                if let Some(token_range) = cursor_token_range {
                    return Some((lambda, param_name, binder_range, token_range));
                }
            }
        }
    }

    None
}

/// Collect all byte-ranges (binder + body uses) for `param_name` inside
/// `lambda`, stopping descent into inner lambdas that re-bind the same name.
///
/// Returns edits as `(start_byte, end_byte)` pairs (new_text is applied by
/// the caller).
fn collect_lambda_param_spans(
    lambda: &smelt_parser::ast::Lambda,
    param_name: &str,
    binder_range: smelt_parser::TextRange,
) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();

    // Always include the binder itself.
    spans.push((binder_range.start().into(), binder_range.end().into()));

    // Walk the body and collect every IDENT token equal to param_name,
    // but stop descending into nested LAMBDAs that shadow the name.
    if let Some(body) = lambda.body() {
        collect_idents_in_body(body.syntax(), param_name, &mut spans);
    }

    spans
}

/// Recursive body walker.  Does not descend into nested LAMBDAs whose
/// parameter list includes `param_name` (shadowing).
fn collect_idents_in_body(
    node: &smelt_parser::syntax_kind::SyntaxNode,
    param_name: &str,
    out: &mut Vec<(usize, usize)>,
) {
    // Collect token-level hits at this node.
    for token in node.children_with_tokens().filter_map(|e| e.into_token()) {
        if token.kind() == SyntaxKind::IDENT && token.text() == param_name {
            let s: usize = token.text_range().start().into();
            let e: usize = token.text_range().end().into();
            out.push((s, e));
        }
    }

    // Descend into child nodes, but skip inner LAMBDAs that shadow the param.
    for child_node in node.children() {
        if child_node.kind() == SyntaxKind::LAMBDA {
            if let Some(inner_lambda) = smelt_parser::ast::Lambda::cast(child_node.clone()) {
                if inner_lambda.params().iter().any(|p| p == param_name) {
                    // Inner lambda shadows param_name — do not descend.
                    continue;
                }
            }
        }
        collect_idents_in_body(&child_node, param_name, out);
    }
}

/// Return the set of outer-scope binder names that are referenced inside the
/// body of `lambda` (but not rebound by an inner lambda in between).
fn outer_binders_referenced_in_body(
    lambda: &smelt_parser::ast::Lambda,
    file: &AstFile,
    text: &str,
) -> std::collections::HashSet<String> {
    // Determine the byte range of this lambda.
    let lambda_start: usize = lambda.syntax().text_range().start().into();
    let lambda_end: usize = lambda.syntax().text_range().end().into();

    // Collect all ancestor LAMBDA nodes (those whose span strictly contains
    // this lambda's span).  Their parameter names are candidates for outer
    // binders.
    let outer_params: Vec<String> = file
        .syntax()
        .descendants()
        .filter(|n| n.kind() == SyntaxKind::LAMBDA)
        .filter(|n| {
            let s: usize = n.text_range().start().into();
            let e: usize = n.text_range().end().into();
            s <= lambda_start && e >= lambda_end && (s < lambda_start || e > lambda_end)
        })
        .filter_map(smelt_parser::ast::Lambda::cast)
        .flat_map(|lam| lam.params())
        .collect();

    if outer_params.is_empty() {
        return std::collections::HashSet::new();
    }

    // Now find which outer params are actually referenced in this lambda's body.
    let mut referenced = std::collections::HashSet::new();
    if let Some(body) = lambda.body() {
        let body_text_range = body.syntax().text_range();
        let body_start: usize = body_text_range.start().into();
        let body_end: usize = body_text_range.end().into();
        let _ = text; // text param reserved for future use

        collect_referenced_names_in_node(body.syntax(), &outer_params, &mut referenced);
        let _ = (body_start, body_end);
    }
    referenced
}

fn collect_referenced_names_in_node(
    node: &smelt_parser::syntax_kind::SyntaxNode,
    candidates: &[String],
    out: &mut std::collections::HashSet<String>,
) {
    // Check token-level hits.
    for token in node.children_with_tokens().filter_map(|e| e.into_token()) {
        if token.kind() == SyntaxKind::IDENT {
            let text = token.text().to_string();
            if candidates.contains(&text) {
                out.insert(text);
            }
        }
    }

    // Don't stop at inner lambdas — outer binders that are used inside inner
    // lambdas still count as "referenced".
    for child_node in node.children() {
        collect_referenced_names_in_node(&child_node, candidates, out);
    }
}

// ─────────────────────────── public API ─────────────────────────────────────

/// Attempt to rename the lambda parameter at `cursor_offset` in `file` to
/// `new_name`.
///
/// Returns:
/// - `Ok(Edits(v))` — success; apply the edits to get the renamed source.
/// - `Ok(NotALambdaParam)` — the cursor is not on a lambda parameter; this
///   handler does not apply.
/// - `Err(e)` — the new name was rejected by a validation rule.
pub fn rename_lambda_param(
    file: &AstFile,
    text: &str,
    cursor_offset: usize,
    new_name: &str,
) -> Result<RenameLambdaResult, RenameLambdaError> {
    // Validate new_name first (before doing any AST work).
    if !smelt_parser::is_valid_sql_identifier(new_name) {
        return Err(RenameLambdaError::InvalidIdentifier(new_name.to_string()));
    }
    if is_meta_keyword(new_name) {
        return Err(RenameLambdaError::ReservedKeyword(new_name.to_string()));
    }

    // Locate the lambda + param at the cursor.
    let (lambda, param_name, binder_range, _cursor_token_range) =
        match find_lambda_and_binder_for_cursor(file, cursor_offset) {
            Some(r) => r,
            None => return Ok(RenameLambdaResult::NotALambdaParam),
        };

    // Finding 1: Check that new_name does not collide with a sibling parameter
    // in the same binder list.  Renaming `b` → `a` in `fn (a, b) => ...`
    // would produce `fn (a, a) => ...`, which silently changes semantics.
    if lambda
        .params()
        .iter()
        .filter(|p| p.as_str() != param_name.as_str())
        .any(|p| p.as_str() == new_name)
    {
        return Err(RenameLambdaError::ShadowsOuterBinder(new_name.to_string()));
    }

    // Check that new_name would not shadow an outer binder already referenced
    // in this lambda's body.
    let outer_refs = outer_binders_referenced_in_body(&lambda, file, text);
    if outer_refs.contains(new_name) {
        return Err(RenameLambdaError::ShadowsOuterBinder(new_name.to_string()));
    }

    // Collect all spans to rename (binder + body uses, excluding inner shadows).
    let spans = collect_lambda_param_spans(&lambda, &param_name, binder_range);

    let edits: Vec<ByteEdit> = spans
        .into_iter()
        .map(|(start, end)| (start, end, new_name.to_string()))
        .collect();

    Ok(RenameLambdaResult::Edits(edits))
}

/// Attempt to identify the lambda parameter binder at `cursor_offset` for a
/// prepare-rename request.
///
/// Returns `Some((start_byte, end_byte, placeholder))` when the cursor is on a
/// binder or body-use of a lambda parameter, or `None` otherwise.
pub fn prepare_rename_lambda_param(
    file: &AstFile,
    _text: &str,
    cursor_offset: usize,
) -> Option<(usize, usize, String)> {
    let (_lambda, param_name, _binder_range, (cursor_start, cursor_end)) =
        find_lambda_and_binder_for_cursor(file, cursor_offset)?;
    // Finding 3: return the cursor-token's span, not always the binder's span.
    // This ensures prepareRename highlights the token the user is hovering over.
    Some((cursor_start, cursor_end, param_name))
}

/// Convert the byte-range edits returned by [`rename_lambda_param`] to LSP
/// `(start_line, start_col, end_line, end_col)` tuples for the backend.
pub fn byte_edits_to_lsp_ranges(text: &str, edits: Vec<ByteEdit>) -> Vec<(u32, u32, u32, u32)> {
    use smelt_parser::TextRange;
    edits
        .into_iter()
        .map(|(start, end, _new_text)| {
            let range = TextRange::new((start as u32).into(), (end as u32).into());
            let r = text_range_to_range(text, range);
            (r.start.line, r.start.column, r.end.line, r.end.column)
        })
        .collect()
}
