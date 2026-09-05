//! Pre-print refusal of constructs the registry declares unsupported.
//!
//! The printer has no diagnostic channel — `print` returns a plain `String`, so
//! a construct the target dialect cannot express would otherwise reach the
//! warehouse and fail there. This module is the pure check the compile path
//! runs *before* printing, turning an engine-side error into a compile-time
//! diagnostic that names both the construct and the backend.
//!
//! Single ownership holds: the verdict is `BuiltinRegistry` data
//! (`Signature::emission_at`), never a list restated here.

use smelt_parser::ast::BinaryExpr;
use smelt_parser::syntax_kind::{SyntaxKind, SyntaxNode};
use smelt_parser::FunctionCall;
use smelt_parser::{TextRange, TextSize};
use smelt_types::signatures::{Position, RewriteId};
use smelt_types::{BuiltinRegistry, CallFacts, DialectId, SettledEmission};

use crate::position::classify as classify_position;
use crate::restructure::within_group_sort_key;
use crate::SqlDialect;

/// One construct the target dialect cannot express.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsupportedEmission {
    /// The canonical registry name (`"//"`, `"REGEXP_MATCHES"`, …), not the
    /// author's spelling — the diagnostic names what the registry refused.
    pub name: &'static str,
    /// The dialect that cannot express it.
    pub dialect: DialectId,
    /// The registry's reason, verbatim. Written for a user, not a maintainer.
    pub reason: &'static str,
    /// The offending expression's span in the source text.
    ///
    /// A `TextRange`, per the diagnostic range-encoding invariant — conversion
    /// to (line, column) happens once, at the diagnostic boundary.
    pub range: TextRange,
}

/// The node's span with trailing trivia removed.
///
/// A `BINARY_EXPR`'s Rowan range absorbs the whitespace that follows it, so an
/// untrimmed range underlines `a // b ` — one column too wide in an editor.
fn trimmed_range(node: &SyntaxNode) -> TextRange {
    let range = node.text_range();
    let text = node.text().to_string();
    let trailing = text.len() - text.trim_end().len();
    TextRange::new(range.start(), range.end() - TextSize::from(trailing as u32))
}

/// A modifier a template's `{n}` placeholder cannot name — dropping any of
/// these would change the answer (a dropped `DISTINCT` counts duplicates; a
/// dropped `FILTER` aggregates excluded rows), so refusal is the only safe
/// outcome. `docs/specs/multi_backend.md` §"Template emission": "A template
/// applies to a plain positional call, and refuses everything else."
const TEMPLATE_MODIFIER_DISTINCT: &str =
    "this built-in's target spelling is a fixed template over positional arguments; DISTINCT \
     cannot be expressed by a template (a dropped DISTINCT would count duplicates the author \
     excluded) and is refused rather than silently dropped";
const TEMPLATE_MODIFIER_FILTER: &str =
    "this built-in's target spelling is a fixed template over positional arguments; a FILTER \
     (WHERE …) clause cannot be expressed by a template (a dropped FILTER would aggregate rows \
     the author excluded) and is refused rather than silently dropped";
const TEMPLATE_MODIFIER_WITHIN_GROUP: &str =
    "this built-in's target spelling is a fixed template over positional arguments; a WITHIN \
     GROUP (ORDER BY …) clause cannot be expressed by a template and is refused rather than \
     silently dropped";
const TEMPLATE_MODIFIER_ORDER_BY: &str =
    "this built-in's target spelling is a fixed template over positional arguments; an ORDER BY \
     inside the argument list cannot be expressed by a template (a dropped ORDER BY would change \
     which value the aggregate sees first) and is refused rather than silently dropped";
const TEMPLATE_MODIFIER_NULL_TREATMENT: &str =
    "this built-in's target spelling is a fixed template over positional arguments; an IGNORE \
     NULLS/RESPECT NULLS modifier cannot be expressed by a template and is refused rather than \
     silently dropped";
const TEMPLATE_MODIFIER_NAMED_PARAM: &str =
    "this built-in's target spelling is a fixed template over positional arguments; a named \
     (`=>`) argument cannot be expressed by a template, which only substitutes by position, and \
     is refused rather than silently dropped";
const TEMPLATE_MODIFIER_STAR: &str =
    "this built-in's target spelling is a fixed template over positional arguments; a `*` \
     argument cannot be expressed by a template and is refused rather than silently dropped";

/// Is `expr` (an `EXPRESSION` node) wrapping exactly a `STAR` token (`COUNT(*)`'s
/// argument) rather than some other single-token expression?
///
/// The parser wraps a bare `*` argument in a nested `EXPRESSION` (one node
/// per layer of `parse_expression`'s recursive descent, verified empirically
/// against the real parser: `COUNT(*)`'s argument is `EXPRESSION(EXPRESSION(STAR))`,
/// not a single layer), so this peels `EXPRESSION` wrappers down to whatever
/// they ultimately hold rather than checking only one level.
fn is_star_expression(expr: &SyntaxNode) -> bool {
    let mut inner = expr.clone();
    loop {
        let mut children = inner.children_with_tokens();
        let (Some(only), None) = (children.next(), children.next()) else {
            return false;
        };
        if let Some(token) = only.as_token() {
            return token.kind() == SyntaxKind::STAR;
        }
        match only.into_node() {
            Some(n) if n.kind() == SyntaxKind::EXPRESSION => inner = n,
            _ => return false,
        }
    }
}

/// A modifier `node` (a `FUNCTION_CALL`) carries that a template's `{n}`
/// placeholder cannot express, if any.
///
/// Inspects only `node`'s own children and its own `ARG_LIST`'s direct
/// children — never `descendants()` — so a modifier on a *nested* call (e.g.
/// `MOD(COUNT(DISTINCT x), 2)`) is not mistaken for a modifier on `node`
/// itself. `WITHIN GROUP`/`FILTER` are direct children of `FUNCTION_CALL`
/// (siblings of `ARG_LIST`); `DISTINCT`, `IGNORE`/`RESPECT NULLS`, the
/// argument list's own `ORDER BY`, a named parameter, and a `*` argument are
/// direct children of `ARG_LIST`.
fn template_unsupported_modifier(node: &SyntaxNode) -> Option<&'static str> {
    if node
        .children()
        .any(|n| n.kind() == SyntaxKind::WITHIN_GROUP_CLAUSE)
    {
        return Some(TEMPLATE_MODIFIER_WITHIN_GROUP);
    }
    if node
        .children()
        .any(|n| n.kind() == SyntaxKind::FILTER_CLAUSE)
    {
        return Some(TEMPLATE_MODIFIER_FILTER);
    }
    let arg_list = node.children().find(|n| n.kind() == SyntaxKind::ARG_LIST)?;
    for child in arg_list.children_with_tokens() {
        if let Some(token) = child.as_token() {
            if token.kind() == SyntaxKind::DISTINCT_KW {
                return Some(TEMPLATE_MODIFIER_DISTINCT);
            }
        } else if let Some(n) = child.as_node() {
            match n.kind() {
                SyntaxKind::NULL_TREATMENT_CLAUSE => return Some(TEMPLATE_MODIFIER_NULL_TREATMENT),
                SyntaxKind::ORDER_BY_CLAUSE => return Some(TEMPLATE_MODIFIER_ORDER_BY),
                SyntaxKind::NAMED_PARAM => return Some(TEMPLATE_MODIFIER_NAMED_PARAM),
                SyntaxKind::EXPRESSION if is_star_expression(n) => {
                    return Some(TEMPLATE_MODIFIER_STAR)
                }
                _ => {}
            }
        }
    }
    None
}

/// Walk `root` for constructs the registry declares unsupported on `dialect`.
///
/// Pure: no I/O, no printing. A name absent from the registry is *not* reported
/// here — unrecognised functions are `UnrecognizedFunction`'s business, so the
/// two diagnostics cannot double-fire on one construct.
pub fn unsupported_emissions(root: &SyntaxNode, dialect: SqlDialect) -> Vec<UnsupportedEmission> {
    let id = dialect.id();
    root.descendants()
        .filter_map(|node| {
            let (name, position) = match node.kind() {
                SyntaxKind::FUNCTION_CALL => (
                    FunctionCall::cast(node.clone())?.name()?,
                    classify_position(&node, root),
                ),
                // Operators are never a call in window/aggregate position —
                // their verdicts are stated with `Position::Any`.
                SyntaxKind::BINARY_EXPR => {
                    (BinaryExpr::cast(node.clone())?.operator()?, Position::Any)
                }
                _ => return None,
            };
            let sig = BuiltinRegistry::resolve(&name)?;
            // No type context is threaded into this pre-print pass (it runs
            // over the bare source CST); a `Conditional` entry's class-guarded
            // arms are unresolvable here, so this settles with arity alone —
            // the same fail-safe lookup-miss fallback the printer uses. A
            // class-guarded arm that a real type would have resolved away is
            // instead decided by the entry's `otherwise` arm, which the
            // registry's own validation ties to the safe direction
            // (`docs/specs/multi_backend.md` §"Operand-conditional
            // verdicts"). No production entry is `Conditional` yet, so this
            // has no observable effect until phase 7 populates one.
            let arity = match node.kind() {
                SyntaxKind::FUNCTION_CALL => FunctionCall::cast(node.clone())
                    .map(|fc| fc.arguments().len())
                    .unwrap_or(0),
                _ => 2,
            };
            match sig.settle_at(id, position, &CallFacts::unresolved(arity)) {
                SettledEmission::Unsupported { reason } => Some(UnsupportedEmission {
                    name: sig.name.as_str(),
                    dialect: id,
                    reason,
                    range: trimmed_range(&node),
                }),
                // A template applies to a plain positional call only — a
                // call carrying a modifier a `{n}` placeholder cannot name is
                // refused here, before the printer ever runs. An operator
                // `BINARY_EXPR` can carry none of these modifiers, so this
                // only ever fires for a `FUNCTION_CALL`.
                SettledEmission::Template(_) if node.kind() == SyntaxKind::FUNCTION_CALL => {
                    template_unsupported_modifier(&node).map(|reason| UnsupportedEmission {
                        name: sig.name.as_str(),
                        dialect: id,
                        reason,
                        range: trimmed_range(&node),
                    })
                }
                // `WithinGroupToAnalytic` rewrites a call whose own `OVER`
                // clause already covers the whole partition — the position
                // check above already admits it — but the source's own
                // `WITHIN GROUP` clause shape still has to be readable: a
                // missing sort key or a `NULLS FIRST`/`LAST` modifier the
                // analytic form cannot express is refused here, before the
                // printer ever runs, rather than reaching
                // `print_within_group_to_analytic`'s verbatim fallback.
                SettledEmission::Rewrite(RewriteId::WithinGroupToAnalytic) => {
                    match within_group_sort_key(&node) {
                        Ok(_) => None,
                        Err(reason) => Some(UnsupportedEmission {
                            name: sig.name.as_str(),
                            dialect: id,
                            reason,
                            range: trimmed_range(&node),
                        }),
                    }
                }
                _ => None,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn only_function_call(sql: &str) -> SyntaxNode {
        smelt_parser::parse(sql)
            .syntax()
            .descendants()
            .find(|n| n.kind() == SyntaxKind::FUNCTION_CALL)
            .expect("expected a FUNCTION_CALL in the fixture")
    }

    #[test]
    fn distinct_argument_is_refused() {
        let call = only_function_call("SELECT COUNT(DISTINCT x) FROM t");
        assert_eq!(
            template_unsupported_modifier(&call),
            Some(TEMPLATE_MODIFIER_DISTINCT)
        );
    }

    #[test]
    fn filter_clause_is_refused() {
        let call = only_function_call("SELECT COUNT(x) FILTER (WHERE y > 0) FROM t");
        assert_eq!(
            template_unsupported_modifier(&call),
            Some(TEMPLATE_MODIFIER_FILTER)
        );
    }

    #[test]
    fn within_group_is_refused() {
        let call =
            only_function_call("SELECT PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY x) FROM t");
        assert_eq!(
            template_unsupported_modifier(&call),
            Some(TEMPLATE_MODIFIER_WITHIN_GROUP)
        );
    }

    #[test]
    fn argument_list_order_by_is_refused() {
        let call = only_function_call("SELECT STRING_AGG(x, ',' ORDER BY x) FROM t");
        assert_eq!(
            template_unsupported_modifier(&call),
            Some(TEMPLATE_MODIFIER_ORDER_BY)
        );
    }

    #[test]
    fn null_treatment_is_refused() {
        let call = only_function_call("SELECT LAST_VALUE(x IGNORE NULLS) FROM t");
        assert_eq!(
            template_unsupported_modifier(&call),
            Some(TEMPLATE_MODIFIER_NULL_TREATMENT)
        );
    }

    #[test]
    fn named_argument_is_refused() {
        let call = only_function_call("SELECT f(a => 1) FROM t");
        assert_eq!(
            template_unsupported_modifier(&call),
            Some(TEMPLATE_MODIFIER_NAMED_PARAM)
        );
    }

    #[test]
    fn star_argument_is_refused() {
        let call = only_function_call("SELECT COUNT(*) FROM t");
        assert_eq!(
            template_unsupported_modifier(&call),
            Some(TEMPLATE_MODIFIER_STAR)
        );
    }

    #[test]
    fn a_plain_positional_call_is_admitted() {
        let call = only_function_call("SELECT MOD(a, b + 1) FROM t");
        assert_eq!(template_unsupported_modifier(&call), None);
    }

    #[test]
    fn a_modifier_on_a_nested_call_does_not_refuse_the_outer_call() {
        // `descendants()` is what `FunctionCall::named_params` uses, which
        // would misattribute the nested COUNT's DISTINCT to the outer MOD
        // call. `only_function_call` returns the first FUNCTION_CALL in
        // preorder — the outer `MOD` call — so this exercises exactly that
        // trap.
        let call = only_function_call("SELECT MOD(COUNT(DISTINCT x), 2) FROM t");
        assert_eq!(template_unsupported_modifier(&call), None);
    }

    #[test]
    fn an_over_clause_is_not_a_modifier() {
        let call = only_function_call("SELECT SUM(x) OVER (PARTITION BY g) FROM t");
        assert_eq!(template_unsupported_modifier(&call), None);
    }
}
