//! `smelt.<path>(...)` call expansion — struct-returning call `.*`
//! projection and the table-expression alias bookkeeping around it.

use smelt_parser::ast::SmeltPathCall;
use smelt_parser::syntax_kind::{SyntaxElement, SyntaxKind, SyntaxNode};

use super::print_node;
use super::PrintContext;

/// Expand a `SMELT_PATH_CALL_STAR` node (`smelt.<path>(args).*`) to
/// per-field projections `<expr> AS <name>, …`.
///
/// Steps:
/// 1. Find the inner `SMELT_PATH_CALL` child and ask the expander for the body.
/// 2. Re-parse the body and find a top-level `BRACE_STRUCT_LITERAL`.
/// 3. Emit each `STRUCT_FIELD_ITEM` verbatim (already `<expr> AS <name>` form).
///
/// Returns `None` when any step fails (no expander, expander returned `None`,
/// body has no brace-struct literal, or body has a `SPREAD_ITEM`), so the
/// caller falls back to verbatim printing.
pub(crate) fn expand_smelt_path_call_star(node: &SyntaxNode, ctx: &PrintContext) -> Option<String> {
    let expander = ctx.smelt_path_call.as_ref()?;

    // Locate the SMELT_PATH_CALL child node.
    let call_node = node
        .children()
        .find(|c| c.kind() == SyntaxKind::SMELT_PATH_CALL)?;

    let path_call = SmeltPathCall::cast(call_node.clone())?;
    let segs = path_call.segments();
    let positional: Vec<String> = path_call
        .arg_list()
        .map(|al| {
            al.positional_args()
                .into_iter()
                .map(|arg| {
                    let mut s = String::new();
                    print_node(arg.syntax(), ctx, &mut s);
                    s
                })
                .collect()
        })
        .unwrap_or_default();
    let named: Vec<(String, String)> = path_call
        .arg_list()
        .map(|al| {
            al.named_params()
                .filter_map(|np| {
                    let name = np.name()?;
                    let expr = np.value_expr()?;
                    let mut s = String::new();
                    print_node(expr.syntax(), ctx, &mut s);
                    Some((name, s))
                })
                .collect()
        })
        .unwrap_or_default();

    let body = expander(&segs, positional, named)?;

    // Re-parse the body to locate a BRACE_STRUCT_LITERAL.
    //
    // The body is an expression fragment (e.g. `{expr AS name, …}`), not a
    // full SQL statement.  `smelt_parser::parse` calls `parse_file`, which
    // only recognises SELECT/WITH/smelt.define at the top level, so a bare
    // brace-struct literal `{…}` would hit the error path and not produce a
    // `BRACE_STRUCT_LITERAL` node.  Wrapping the body in a minimal
    // `SELECT <body>` forces expression parsing and places the struct literal
    // inside a `SELECT_ITEM → EXPRESSION` wrapper where the traversal below
    // can find it.
    let wrapper = format!("SELECT {body}");
    let reparsed = smelt_parser::parse(&wrapper);
    let syntax_root = reparsed.syntax();

    // Walk to find the BRACE_STRUCT_LITERAL node anywhere in the tree.
    let brace_struct = find_brace_struct_literal(&syntax_root)?;

    // Reject if any SPREAD_ITEM is present — fall back to verbatim.
    let has_spread = brace_struct
        .children()
        .any(|c| c.kind() == SyntaxKind::SPREAD_ITEM);
    if has_spread {
        return None;
    }

    // Collect STRUCT_FIELD_ITEM children and emit them as separate projections.
    let fields: Vec<SyntaxNode> = brace_struct
        .children()
        .filter(|c| c.kind() == SyntaxKind::STRUCT_FIELD_ITEM)
        .collect();

    if fields.is_empty() {
        return None;
    }

    let mut out = String::new();
    for (i, field) in fields.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        // Each STRUCT_FIELD_ITEM is `<expr> AS <name>` — print it verbatim
        // (running it through print_node applies further expansions if needed).
        print_node(field, ctx, &mut out);
    }
    Some(out)
}

/// Check whether the `TABLE_REF` parent of a `SMELT_PATH_CALL` carries a
/// user-supplied alias.
///
/// The parser places a `SMELT_PATH_CALL` inside `TABLE_REF` and then, if the
/// user wrote `AS alias` or a bare implicit alias, appends those tokens as
/// siblings in the same `TABLE_REF` node.  We scan the children of
/// `table_ref` and look for any `AS_KW` or `IDENT` token that appears after
/// the `SMELT_PATH_CALL` child.
pub(crate) fn smelt_path_call_has_explicit_alias(
    table_ref: &SyntaxNode,
    call_node: &SyntaxNode,
) -> bool {
    let call_range = call_node.text_range();
    let mut past_call = false;
    for child in table_ref.children_with_tokens() {
        match &child {
            SyntaxElement::Node(n) => {
                if n.text_range() == call_range {
                    past_call = true;
                }
            }
            SyntaxElement::Token(t) => {
                if past_call && !t.kind().is_trivia() {
                    // Any non-trivia token after the SMELT_PATH_CALL is either
                    // AS_KW (explicit alias) or IDENT (implicit alias).  Both
                    // indicate a user-supplied alias.
                    if matches!(t.kind(), SyntaxKind::AS_KW | SyntaxKind::IDENT) {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// Recursively find the first `BRACE_STRUCT_LITERAL` node in the tree,
/// descending through FILE and EXPRESSION wrapper nodes.
fn find_brace_struct_literal(node: &SyntaxNode) -> Option<SyntaxNode> {
    if node.kind() == SyntaxKind::BRACE_STRUCT_LITERAL {
        return Some(node.clone());
    }
    for child in node.children() {
        if let Some(found) = find_brace_struct_literal(&child) {
            return Some(found);
        }
    }
    None
}
