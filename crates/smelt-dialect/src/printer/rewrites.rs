//! Capability-gated expression rewrites — the small, local emission
//! differences a `BackendCapabilities` flag decides (`::` casts, `ARRAY[..]`
//! literals, `QUALIFY`, trailing commas), plus the shared child walk.

use smelt_parser::syntax_kind::{SyntaxElement, SyntaxKind, SyntaxNode};
use smelt_parser::CastExpr;

use super::print_node;
use super::PrintContext;

/// Walk children with index-based iteration, allowing look-ahead for DATE literal rewrite.
pub(crate) fn print_children(node: &SyntaxNode, ctx: &PrintContext, out: &mut String) {
    let children: Vec<SyntaxElement> = node.children_with_tokens().collect();
    let mut i = 0;
    while i < children.len() {
        match &children[i] {
            SyntaxElement::Token(token) => {
                // DATE literal rewrite: DATE 'value' → DATE('value')
                if !ctx.capabilities.supports_date_literal
                    && token.kind() == SyntaxKind::IDENT
                    && token.text().eq_ignore_ascii_case("DATE")
                {
                    if let Some((skip_to, string_text)) = find_string_after(&children, i + 1) {
                        out.push_str("DATE(");
                        out.push_str(&string_text);
                        out.push(')');
                        i = skip_to + 1;
                        continue;
                    }
                }
                out.push_str(token.text());
            }
            SyntaxElement::Node(child_node) => {
                print_node(child_node, ctx, out);
            }
        }
        i += 1;
    }
}

/// Look ahead in children for optional whitespace followed by a STRING token.
/// Returns (index_of_string, string_text) if found.
fn find_string_after(children: &[SyntaxElement], start: usize) -> Option<(usize, String)> {
    let mut j = start;
    while j < children.len() {
        match &children[j] {
            SyntaxElement::Token(t) if t.kind() == SyntaxKind::WHITESPACE => {
                j += 1;
            }
            SyntaxElement::Token(t) if t.kind() == SyntaxKind::STRING => {
                return Some((j, t.text().to_string()));
            }
            _ => return None,
        }
    }
    None
}

/// Rewrite expr::type → CAST(expr AS type) when backend doesn't support ::.
/// If it's already CAST(...) syntax, pass through verbatim.
pub(crate) fn print_cast_rewrite(node: &SyntaxNode, ctx: &PrintContext, out: &mut String) {
    let Some(cast) = CastExpr::cast(node.clone()) else {
        print_children(node, ctx, out);
        return;
    };

    if !cast.is_double_colon_cast() {
        // Already CAST(expr AS type) syntax — pass through
        print_children(node, ctx, out);
        return;
    }

    // Partition children into: expr (before ::), type (TYPE_SPEC node), trailing whitespace.
    // We emit CAST(expr AS type) followed by any trailing whitespace.
    let children: Vec<SyntaxElement> = node.children_with_tokens().collect();

    // Find the :: token index
    let dc_idx = children
        .iter()
        .position(|c| matches!(c, SyntaxElement::Token(t) if t.kind() == SyntaxKind::DOUBLE_COLON));
    let Some(dc_idx) = dc_idx else {
        print_children(node, ctx, out);
        return;
    };

    // Find the TYPE_SPEC node index
    let type_idx = children
        .iter()
        .position(|c| matches!(c, SyntaxElement::Node(n) if n.kind() == SyntaxKind::TYPE_SPEC));

    out.push_str("CAST(");

    // Print expression (children before ::)
    for child in &children[..dc_idx] {
        match child {
            SyntaxElement::Token(t) => out.push_str(t.text()),
            SyntaxElement::Node(n) => print_node(n, ctx, out),
        }
    }

    out.push_str(" AS ");

    // Print TYPE_SPEC, moving any trailing whitespace outside the closing paren
    let mut type_text = String::new();
    if let Some(ti) = type_idx {
        if let SyntaxElement::Node(n) = &children[ti] {
            print_node(n, ctx, &mut type_text);
        }
    }
    let trimmed = type_text.trim_end();
    let trailing = &type_text[trimmed.len()..];
    out.push_str(trimmed);
    out.push(')');
    out.push_str(trailing);

    // Print any remaining children after TYPE_SPEC (unlikely but defensive)
    let after = type_idx.map(|ti| ti + 1).unwrap_or(children.len());
    for child in &children[after..] {
        match child {
            SyntaxElement::Token(t) => out.push_str(t.text()),
            SyntaxElement::Node(n) => print_node(n, ctx, out),
        }
    }
}

/// Rewrite ARRAY[1,2,3] → ARRAY(1,2,3).
pub(crate) fn print_array_rewrite(node: &SyntaxNode, ctx: &PrintContext, out: &mut String) {
    for child in node.children_with_tokens() {
        match child {
            SyntaxElement::Token(token) => match token.kind() {
                SyntaxKind::LBRACKET => out.push('('),
                SyntaxKind::RBRACKET => out.push(')'),
                _ => out.push_str(token.text()),
            },
            SyntaxElement::Node(child_node) => {
                print_node(&child_node, ctx, out);
            }
        }
    }
}

/// Handle SELECT with QUALIFY → subquery rewrite when backend doesn't support QUALIFY.
pub(crate) fn print_select_with_qualify_rewrite(
    node: &SyntaxNode,
    ctx: &PrintContext,
    out: &mut String,
) {
    let has_qualify = node
        .children()
        .any(|c| c.kind() == SyntaxKind::QUALIFY_CLAUSE);

    if !has_qualify {
        print_children(node, ctx, out);
        return;
    }

    // Extract the QUALIFY expression
    let qualify_expr = node
        .children()
        .find(|c| c.kind() == SyntaxKind::QUALIFY_CLAUSE)
        .and_then(|qc| {
            let mut found_kw = false;
            let mut expr_parts = Vec::new();
            for child in qc.children_with_tokens() {
                match child {
                    SyntaxElement::Token(t) => {
                        if t.kind() == SyntaxKind::QUALIFY_KW {
                            found_kw = true;
                        } else if found_kw {
                            expr_parts.push(t.text().to_string());
                        }
                    }
                    SyntaxElement::Node(n) => {
                        if found_kw {
                            let mut s = String::new();
                            print_node(&n, ctx, &mut s);
                            expr_parts.push(s);
                        }
                    }
                }
            }
            let trimmed = expr_parts.join("").trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        });

    let Some(qualify_expr) = qualify_expr else {
        print_children(node, ctx, out);
        return;
    };

    // Wrap: SELECT * FROM (inner_select_without_qualify) _q WHERE qualify_expr
    out.push_str("SELECT * FROM (");
    print_children_skip_qualify(node, ctx, out);
    out.push_str(") _q WHERE ");
    out.push_str(&qualify_expr);
}

/// Print a SELECT statement's children, skipping the QUALIFY clause.
fn print_children_skip_qualify(node: &SyntaxNode, ctx: &PrintContext, out: &mut String) {
    let children: Vec<SyntaxElement> = node.children_with_tokens().collect();
    let mut i = 0;
    while i < children.len() {
        match &children[i] {
            SyntaxElement::Token(token) => {
                if !ctx.capabilities.supports_date_literal
                    && token.kind() == SyntaxKind::IDENT
                    && token.text().eq_ignore_ascii_case("DATE")
                {
                    if let Some((skip_to, string_text)) = find_string_after(&children, i + 1) {
                        out.push_str("DATE(");
                        out.push_str(&string_text);
                        out.push(')');
                        i = skip_to + 1;
                        continue;
                    }
                }
                out.push_str(token.text());
            }
            SyntaxElement::Node(child_node) => {
                if child_node.kind() == SyntaxKind::QUALIFY_CLAUSE {
                    i += 1;
                    continue;
                }
                print_node(child_node, ctx, out);
            }
        }
        i += 1;
    }
}

/// Print children of a SELECT_LIST or GROUP_BY_CLAUSE, stripping trailing commas.
pub(crate) fn print_strip_trailing_commas(node: &SyntaxNode, ctx: &PrintContext, out: &mut String) {
    let children: Vec<SyntaxElement> = node.children_with_tokens().collect();
    for (i, child) in children.iter().enumerate() {
        match child {
            SyntaxElement::Token(token) if token.kind() == SyntaxKind::COMMA => {
                // Look ahead: is there any non-whitespace child after this comma?
                let has_more = children[i + 1..].iter().any(
                    |c| !matches!(c, SyntaxElement::Token(t) if t.kind() == SyntaxKind::WHITESPACE),
                );
                if has_more {
                    out.push_str(token.text());
                }
                // else: trailing comma — skip it (but keep any following whitespace)
            }
            SyntaxElement::Token(token) => {
                out.push_str(token.text());
            }
            SyntaxElement::Node(child_node) => {
                print_node(child_node, ctx, out);
            }
        }
    }
}
