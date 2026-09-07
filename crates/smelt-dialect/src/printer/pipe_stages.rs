//! Per-stage body collectors for pipe-syntax lowering: the text of one
//! `|>` stage (WHERE, SELECT, ORDER BY, aggregate, LIMIT, JOIN, …) read out
//! of its CST node, plus the small comma-list parsers the SET/DROP/RENAME
//! stages hand their bodies to.

use smelt_parser::syntax_kind::{SyntaxElement, SyntaxKind, SyntaxNode};

use super::print_node;
use super::PrintContext;

/// Collect the FROM body (everything after the FROM keyword) as a printed string.
pub(super) fn collect_from_body(from_clause: &SyntaxNode, ctx: &PrintContext) -> String {
    use SyntaxKind::*;

    let mut body = String::new();
    let mut past_from_kw = false;
    for child_elem in from_clause.children_with_tokens() {
        match child_elem {
            SyntaxElement::Token(t) => {
                if t.kind() == FROM_KW {
                    past_from_kw = true;
                    continue;
                }
                if past_from_kw {
                    body.push_str(t.text());
                }
            }
            SyntaxElement::Node(n) => {
                if past_from_kw {
                    print_node(&n, ctx, &mut body);
                }
            }
        }
    }
    body.trim().to_string()
}

/// Collect the body text of a stage after its operator marker, running nodes
/// through `print_node` for smelt-path expansion.
///
/// The CST for contextual-keyword stages (EXTEND, SET, DROP, RENAME) has:
///   PIPE_STAGE { PIPE_OP_<X> [zero-width marker]  IDENT("<keyword>")  ... body ... }
///
/// We set `past_op` after the zero-width marker node, then skip one more IDENT
/// that is the operator keyword itself (e.g. "EXTEND", "SET"), and also any
/// immediately following whitespace, before appending body content.
pub(super) fn collect_stage_body_text(
    stage: &SyntaxNode,
    ctx: &PrintContext,
    op_kind: SyntaxKind,
) -> String {
    use SyntaxKind::*;

    let mut past_op = false;
    let mut skipped_keyword = false;
    let mut body = String::new();

    for elem in stage.children_with_tokens() {
        match &elem {
            SyntaxElement::Node(n) => {
                if n.kind() == op_kind {
                    past_op = true;
                    continue;
                }
                if past_op {
                    print_node(n, ctx, &mut body);
                }
            }
            SyntaxElement::Token(t) => {
                if !past_op {
                    continue;
                }
                if !skipped_keyword {
                    // Skip the operator keyword IDENT (e.g. "EXTEND", "SET").
                    if t.kind() == IDENT {
                        skipped_keyword = true;
                        continue;
                    }
                    // Skip trivia (whitespace) before we've seen the keyword IDENT.
                    if t.kind().is_trivia() {
                        continue;
                    }
                    // Anything else: keyword was implicit/absent, start body here.
                    skipped_keyword = true;
                    body.push_str(t.text());
                } else {
                    body.push_str(t.text());
                }
            }
        }
    }
    body.trim().to_string()
}

/// Extract the alias name from a PIPE_OP_AS stage.
pub(super) fn collect_as_alias(stage: &SyntaxNode) -> Option<String> {
    use SyntaxKind::*;
    let mut past_op = false;
    for elem in stage.children_with_tokens() {
        match &elem {
            SyntaxElement::Node(n) if n.kind() == PIPE_OP_AS => {
                past_op = true;
            }
            SyntaxElement::Token(t) if past_op && !t.kind().is_trivia() && t.kind() == IDENT => {
                return Some(t.text().to_string());
            }
            _ => {}
        }
    }
    None
}

/// Collect the body text for a WHERE pipe stage.
///
/// The PIPE_STAGE for WHERE contains:
/// - `PIPE_OP_WHERE` marker (zero-width node)
/// - `WHERE_KW` token
/// - whitespace
/// - `EXPRESSION` node (the predicate)
///
/// We skip the marker and the WHERE keyword and return the predicate text.
pub(super) fn collect_where_body(stage: &SyntaxNode) -> String {
    use SyntaxKind::*;

    let mut past_op = false;
    let mut body = String::new();

    for elem in stage.children_with_tokens() {
        match &elem {
            SyntaxElement::Node(n) => {
                let k = n.kind();
                if matches!(k, PIPE_OP_WHERE) {
                    // Skip zero-width marker
                    continue;
                }
                // Any other node (EXPRESSION, etc.) is body content.
                past_op = true;
                body.push_str(&n.text().to_string());
            }
            SyntaxElement::Token(t) => {
                if !past_op {
                    // Skip WHERE_KW and whitespace before the predicate.
                    if matches!(t.kind(), WHERE_KW | WHITESPACE) {
                        continue;
                    }
                    // First non-keyword, non-whitespace token → body starts.
                    past_op = true;
                }
                body.push_str(t.text());
            }
        }
    }

    body.trim().to_string()
}

/// Collect the SELECT list body for a SELECT pipe stage.
///
/// The PIPE_STAGE for SELECT contains:
/// - `PIPE_OP_SELECT` marker (zero-width node)
/// - `SELECT_KW` token
/// - whitespace
/// - `SELECT_LIST` node (the projection list)
///
/// We skip the marker and SELECT keyword and return the list text.
pub(super) fn collect_select_body(stage: &SyntaxNode) -> String {
    use SyntaxKind::*;

    let mut past_kw = false;
    let mut body = String::new();

    for elem in stage.children_with_tokens() {
        match &elem {
            SyntaxElement::Node(n) => {
                let k = n.kind();
                if matches!(k, PIPE_OP_SELECT) {
                    continue;
                }
                // SELECT_LIST or any other node → body
                past_kw = true;
                body.push_str(&n.text().to_string());
            }
            SyntaxElement::Token(t) => {
                if !past_kw {
                    if matches!(t.kind(), SELECT_KW | WHITESPACE) {
                        continue;
                    }
                    past_kw = true;
                }
                body.push_str(t.text());
            }
        }
    }

    body.trim().to_string()
}

/// Collect the ORDER BY body for an ORDER BY pipe stage.
///
/// The PIPE_STAGE for ORDER BY contains:
/// - `PIPE_OP_ORDER_BY` marker (zero-width node)
/// - `ORDER_BY_CLAUSE` node (which itself starts with ORDER_KW BY_KW)
///
/// We emit the text of the ORDER_BY_CLAUSE *after* stripping "ORDER BY " prefix.
pub(super) fn collect_order_by_body(stage: &SyntaxNode) -> String {
    use SyntaxKind::*;

    // Find the ORDER_BY_CLAUSE child node.
    for child in stage.children() {
        if child.kind() == ORDER_BY_CLAUSE {
            // The ORDER_BY_CLAUSE text starts with "ORDER BY …".
            // We want just the items list (everything after ORDER BY).
            // Strip leading "ORDER BY " (case-insensitive, may have varying whitespace).
            // We do this by scanning past the ORDER + BY tokens.
            let mut past_by = false;
            let mut body = String::new();
            for elem in child.children_with_tokens() {
                match &elem {
                    SyntaxElement::Token(t) => {
                        if !past_by {
                            if matches!(t.kind(), ORDER_KW | BY_KW | WHITESPACE) {
                                if t.kind() == BY_KW {
                                    past_by = true;
                                }
                                continue;
                            }
                            past_by = true;
                        }
                        body.push_str(t.text());
                    }
                    SyntaxElement::Node(n) => {
                        if past_by {
                            body.push_str(&n.text().to_string());
                        }
                    }
                }
            }
            return body.trim().to_string();
        }
    }

    // Fallback: return raw text if no ORDER_BY_CLAUSE found.
    stage.text().to_string().trim().to_string()
}

/// Collect the aggregate expressions and group-by expressions from a PIPE_OP_AGGREGATE stage.
///
/// Returns `(agg_body, group_by_body)` where:
/// - `agg_body` is the comma-separated aggregate expression list (with aliases), as printed SQL.
/// - `group_by_body` is the comma-separated group-by expression list (with aliases), as printed SQL.
///   Empty string when there is no GROUP BY clause (full-table aggregation).
///
/// CST structure for `|> AGGREGATE sum(x) AS s GROUP BY k`:
///   PIPE_STAGE {
///     PIPE_OP_AGGREGATE (zero-width node)
///     IDENT("AGGREGATE")
///     EXPRESSION(sum(x))
///     AS_KW
///     IDENT("s")
///     GROUP_KW
///     BY_KW
///     EXPRESSION(k)
///   }
pub(super) fn collect_aggregate_parts(stage: &SyntaxNode, ctx: &PrintContext) -> (String, String) {
    use SyntaxKind::*;

    let children: Vec<SyntaxElement> = stage.children_with_tokens().collect();

    // Find GROUP_KW position (if any) to split agg vs group-by portions.
    let group_kw_pos = children
        .iter()
        .position(|elem| matches!(elem, SyntaxElement::Token(t) if t.kind() == GROUP_KW));

    // Determine end of agg portion (exclusive).
    let agg_end = group_kw_pos.unwrap_or(children.len());

    // Find where the agg portion starts: past PIPE_OP_AGGREGATE node and the "AGGREGATE" IDENT.
    let mut agg_start = 0;
    // Skip PIPE_OP_AGGREGATE zero-width node.
    for (i, elem) in children.iter().enumerate() {
        if let SyntaxElement::Node(n) = elem {
            if n.kind() == PIPE_OP_AGGREGATE {
                agg_start = i + 1;
                break;
            }
        }
    }
    // Skip trivia and then the "AGGREGATE" contextual keyword IDENT.
    while agg_start < agg_end {
        match &children[agg_start] {
            SyntaxElement::Token(t) if t.kind().is_trivia() => agg_start += 1,
            SyntaxElement::Token(t) if t.kind() == IDENT => {
                agg_start += 1;
                break;
            }
            _ => break,
        }
    }

    // Collect agg portion as printed SQL text.
    let agg_body = collect_elements_as_text(&children[agg_start..agg_end], ctx)
        .trim()
        .to_string();

    // Collect group-by portion (after BY_KW).
    let group_body = if let Some(gkw) = group_kw_pos {
        // Find BY_KW after GROUP_KW.
        let by_pos = children[gkw..]
            .iter()
            .position(|elem| matches!(elem, SyntaxElement::Token(t) if t.kind() == BY_KW));
        if let Some(by_rel) = by_pos {
            let by_abs = gkw + by_rel + 1; // +1 to skip past BY_KW
            collect_elements_as_text(&children[by_abs..], ctx)
                .trim()
                .to_string()
        } else {
            String::new()
        }
    } else {
        String::new()
    };

    (agg_body, group_body)
}

/// Print a slice of CST elements as SQL text, running nodes through `print_node`.
pub(super) fn collect_elements_as_text(elements: &[SyntaxElement], ctx: &PrintContext) -> String {
    let mut out = String::new();
    for elem in elements {
        match elem {
            SyntaxElement::Token(t) => out.push_str(t.text()),
            SyntaxElement::Node(n) => print_node(n, ctx, &mut out),
        }
    }
    out
}

/// Collect the LIMIT value for a LIMIT pipe stage.
///
/// The PIPE_STAGE for LIMIT contains:
/// - `PIPE_OP_LIMIT` marker (zero-width node)
/// - `LIMIT_CLAUSE` node (which itself contains LIMIT_KW and the value)
///
/// We emit the text of the LIMIT_CLAUSE after stripping "LIMIT " prefix.
pub(super) fn collect_limit_body(stage: &SyntaxNode) -> String {
    use SyntaxKind::*;

    for child in stage.children() {
        if child.kind() == LIMIT_CLAUSE {
            // Strip the LIMIT keyword, return just the value (and optional OFFSET).
            let mut past_kw = false;
            let mut body = String::new();
            for elem in child.children_with_tokens() {
                match &elem {
                    SyntaxElement::Token(t) => {
                        if !past_kw {
                            if matches!(t.kind(), LIMIT_KW | WHITESPACE) {
                                if t.kind() == LIMIT_KW {
                                    past_kw = true;
                                }
                                continue;
                            }
                            past_kw = true;
                        }
                        body.push_str(t.text());
                    }
                    SyntaxElement::Node(n) => {
                        if past_kw {
                            body.push_str(&n.text().to_string());
                        }
                    }
                }
            }
            return body.trim().to_string();
        }
    }

    stage.text().to_string().trim().to_string()
}

/// Parse a SET stage body `col = expr, col2 = expr2` into `[(col, expr), ...]`.
///
/// Splits on commas that are not nested inside parentheses, then for each item
/// splits on the first `=` to extract the column name (left-hand side) and
/// expression (right-hand side).  Whitespace is trimmed from both sides.
///
/// If any item lacks a `=`, it is skipped — the caller emits the whole body
/// verbatim in that case (but currently the `has_unhandled` check rejects
/// syntactically broken SET stages before we get here).
pub(super) fn parse_set_assignments(body: &str) -> Vec<(String, String)> {
    split_comma_top_level(body)
        .into_iter()
        .filter_map(|item| {
            let item = item.trim();
            let eq_pos = item.find('=')?;
            let col = item[..eq_pos].trim().to_string();
            let expr = item[eq_pos + 1..].trim().to_string();
            if col.is_empty() || expr.is_empty() {
                None
            } else {
                Some((col, expr))
            }
        })
        .collect()
}

/// Parse a DROP stage body `col1, col2, ...` into `["col1", "col2", ...]`.
pub(super) fn parse_column_list(body: &str) -> Vec<String> {
    split_comma_top_level(body)
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Parse a RENAME stage body `old AS new, old2 AS new2` into `[(old, new), ...]`.
///
/// Each item must contain a case-insensitive ` AS ` separator.  Items that lack
/// it are skipped.
pub(super) fn parse_rename_pairs(body: &str) -> Vec<(String, String)> {
    split_comma_top_level(body)
        .into_iter()
        .filter_map(|item| {
            let item = item.trim();
            // Find " AS " (case-insensitive).
            let upper = item.to_uppercase();
            let as_pos = upper.find(" AS ")?;
            let old = item[..as_pos].trim().to_string();
            let new = item[as_pos + 4..].trim().to_string();
            if old.is_empty() || new.is_empty() {
                None
            } else {
                Some((old, new))
            }
        })
        .collect()
}

/// Split a comma-separated list at the top level (not inside parentheses).
///
/// Tracks paren depth so that expressions like `COALESCE(a, b)` or
/// `DATE_TRUNC('month', ts)` are not split on their internal commas.
pub(super) fn split_comma_top_level(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut depth: u32 = 0;
    let mut current = String::new();
    for ch in s.chars() {
        match ch {
            '(' => {
                depth += 1;
                current.push(ch);
            }
            ')' => {
                depth = depth.saturating_sub(1);
                current.push(ch);
            }
            ',' if depth == 0 => {
                parts.push(current.clone());
                current.clear();
            }
            _ => {
                current.push(ch);
            }
        }
    }
    if !current.trim().is_empty() {
        parts.push(current);
    }
    parts
}

/// Collect the JOIN clause text from a JOIN pipe stage.
///
/// The CST for a JOIN stage is:
///   PIPE_STAGE { PIPE_OP_JOIN (zero-width)  JOIN_CLAUSE { ... } }
///
/// We print the JOIN_CLAUSE node as SQL text and return it trimmed.
pub(super) fn collect_join_clause_text(stage: &SyntaxNode, ctx: &PrintContext) -> String {
    use SyntaxKind::JOIN_CLAUSE;
    for child in stage.children() {
        if child.kind() == JOIN_CLAUSE {
            let mut text = String::new();
            print_node(&child, ctx, &mut text);
            return text.trim().to_string();
        }
    }
    // Fallback: collect everything after the PIPE_OP_JOIN marker.
    collect_stage_body_text(stage, ctx, SyntaxKind::PIPE_OP_JOIN)
}
