//! Row-local/determinism predicates and small expression-shape helpers used
//! by [`super::classify_keyed_succession`].

use smelt_parser::{Expr, SyntaxKind};
use smelt_types::SqlFunction;

use crate::analysis::monotonicity::FunctionDeterminism;

pub(super) fn names_match(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// `expr` is `NOT <inner>` — the unary boolean negation reuses
/// `BINARY_EXPR`'s node kind with no right operand
/// (`smelt_parser::BinaryExpr::operator`'s own doc comment).
pub(super) fn as_bare_not(expr: &Expr) -> Option<Expr> {
    let bin = expr.as_binary()?;
    if bin.operator().as_deref() == Some("NOT") && bin.right().is_none() {
        bin.left()
    } else {
        None
    }
}

/// `expr` is a deterministic row-local predicate: no aggregate, window, or
/// subquery ([`is_row_local`]), and no function anywhere in its subtree
/// classifies as run-deterministic or row-nondeterministic under the
/// determinism predicate (`model_properties.md` §"Determinism (run vs row)
/// and the nondeterminism predicate") — the lateness clamp must be stable
/// across runs.
pub(super) fn is_deterministic_row_local(expr: &Expr) -> bool {
    is_row_local(expr) && !contains_nondeterministic_function(expr)
}

fn contains_nondeterministic_function(expr: &Expr) -> bool {
    expr.syntax()
        .descendants()
        .filter(|n| n.kind() == SyntaxKind::FUNCTION_CALL)
        .filter_map(smelt_parser::FunctionCall::cast)
        .any(|func| {
            let name = func.name().unwrap_or_default();
            !matches!(
                crate::analysis::monotonicity::classify_function_determinism(&name),
                FunctionDeterminism::Neither
            )
        })
}

/// `expr` is a row-local function of the current row alone: no window
/// `OVER`, no subquery, and no aggregate function call anywhere in its
/// subtree.
pub(super) fn is_row_local(expr: &Expr) -> bool {
    if expr
        .syntax()
        .descendants()
        .any(|n| n.kind() == SyntaxKind::WINDOW_SPEC)
    {
        return false;
    }
    if expr
        .syntax()
        .descendants()
        .any(|n| n.kind() == SyntaxKind::SUBQUERY)
    {
        return false;
    }
    !expr
        .syntax()
        .descendants()
        .filter(|n| n.kind() == SyntaxKind::FUNCTION_CALL)
        .filter_map(smelt_parser::FunctionCall::cast)
        .any(|func| {
            let name = func.name().unwrap_or_default().to_uppercase();
            SqlFunction::from_name(&name).is_some_and(|f| f.is_aggregate())
        })
}
