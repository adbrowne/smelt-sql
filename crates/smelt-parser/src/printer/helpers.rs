use super::*;
use crate::syntax_kind::SyntaxNode;

/// Extract GROUP BY expressions from syntax node
pub(super) fn extract_group_by_expressions(node: &SyntaxNode) -> String {
    // DuckDB `GROUP BY ALL`: the clause carries a bare ALL_KW marker and no
    // grouping-key expressions.
    let is_all = node
        .children_with_tokens()
        .filter_map(|e| e.into_token())
        .any(|t| t.kind() == ALL_KW);
    if is_all {
        return "ALL".to_string();
    }

    let mut expressions = Vec::new();
    for child in node.children() {
        if child.kind() == EXPRESSION
            || child.kind() == BINARY_EXPR
            || child.kind() == GROUPING_SETS_CLAUSE
        {
            expressions.push(child.text().to_string());
        }
    }
    expressions.join(", ")
}

/// Info about a set operation (UNION/INTERSECT/EXCEPT)
pub(super) struct SetOperation {
    pub(super) keyword: &'static str,
    pub(super) all: bool,
    pub(super) by_name: bool,
    pub(super) operand: SetOperand,
    /// Source offset of the UNION/INTERSECT/EXCEPT keyword token itself.
    /// Used to decide whether a sibling ORDER BY/LIMIT clause on the same
    /// SELECT_STMT node was parsed *before* the set-op keyword (the
    /// historical position — a per-operand clause on a SELECT_STMT that
    /// happens to also carry a nested set-op tail) or *after* the operand
    /// (a trailing clause on a parenthesized operand — `A UNION (B) ORDER
    /// BY x` — which binds to the whole set operation per DuckDB
    /// semantics; see `parse_set_op_tail` in `parser/select.rs`). Printing
    /// must preserve that positional distinction or it silently
    /// re-attaches the clause to the wrong operand.
    pub(super) keyword_offset: usize,
}

/// The right-hand operand of a set operation: a bare `SELECT_STMT`
/// (`A UNION B`), or a parenthesized `SUBQUERY` (`A UNION (B)`) — printed
/// via `Subquery`'s own Display so the parens round-trip.
pub(super) enum SetOperand {
    None,
    Select(SelectStmt),
    Paren(Subquery),
}

/// Detect and extract set operation (UNION/INTERSECT/EXCEPT) from a SELECT_STMT node
pub(super) fn get_set_operation(node: &SyntaxNode) -> Option<SetOperation> {
    let set_op_kinds = [UNION_KW, INTERSECT_KW, EXCEPT_KW];

    let tokens: Vec<_> = node
        .children_with_tokens()
        .filter_map(|e| e.into_token())
        .collect();

    // Find the set operation keyword
    let mut op_kind = None;
    let mut has_all = false;
    let mut has_by_name = false;
    let mut keyword_offset = 0usize;

    for (i, token) in tokens.iter().enumerate() {
        if set_op_kinds.contains(&token.kind()) {
            op_kind = Some(token.kind());
            keyword_offset = usize::from(token.text_range().start());
            // Check for ALL after the keyword.
            let non_trivia: Vec<_> = tokens[i + 1..]
                .iter()
                .filter(|t| !matches!(t.kind(), WHITESPACE | COMMENT))
                .collect();
            let mut idx = 0;
            if non_trivia.first().is_some_and(|t| t.kind() == ALL_KW) {
                has_all = true;
                idx = 1;
            }
            // Check for BY NAME (DuckDB): `BY_KW` followed by the
            // contextual `NAME` keyword (plain IDENT), matched only as
            // this exact sequence.
            if let (Some(by_tok), Some(name_tok)) = (non_trivia.get(idx), non_trivia.get(idx + 1)) {
                if by_tok.kind() == BY_KW
                    && name_tok.kind() == IDENT
                    && name_tok.text().eq_ignore_ascii_case("NAME")
                {
                    has_by_name = true;
                }
            }
            break;
        }
    }

    let op_kind = op_kind?;

    let keyword = match op_kind {
        UNION_KW => "UNION",
        INTERSECT_KW => "INTERSECT",
        EXCEPT_KW => "EXCEPT",
        _ => unreachable!(),
    };

    // Find the operand after the set operation: either a bare SELECT_STMT
    // or a parenthesized SUBQUERY.
    let mut found_op = false;
    let mut operand = SetOperand::None;
    for child in node.children_with_tokens() {
        if let Some(token) = child.as_token() {
            if token.kind() == op_kind {
                found_op = true;
            }
        } else if found_op {
            if let Some(n) = child.as_node() {
                if n.kind() == SELECT_STMT {
                    if let Some(select) = SelectStmt::cast(n.clone()) {
                        operand = SetOperand::Select(select);
                    }
                    break;
                }
                if n.kind() == SUBQUERY {
                    if let Some(subquery) = Subquery::cast(n.clone()) {
                        operand = SetOperand::Paren(subquery);
                    }
                    break;
                }
            }
        }
    }

    Some(SetOperation {
        keyword,
        all: has_all,
        by_name: has_by_name,
        operand,
        keyword_offset,
    })
}
