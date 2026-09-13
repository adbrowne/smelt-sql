//! Frame elision on offset functions (`docs/specs/multi_backend.md`
//! §"Frame elision on offset functions") — the only place outside `printer/`
//! that knows the `WINDOW_SPEC`/`WINDOW_FRAME` node shapes for this purpose.
//!
//! Decided live, at the point the printer visits a `WINDOW_FRAME` node,
//! rather than planned ahead like `restructure::plan` against the model's
//! own source CST: a `smelt.define` function body is inlined by a textual
//! re-parse at print time (`printer::reexpand_call_body`), so a `LAG`/`LEAD`
//! call inside one never appears in the tree a pre-pass over the model's own
//! `syntax` would walk. Position classification and the registry lookup
//! below are local to whichever tree is currently being printed — the same
//! property `RewriteId::BigQueryMedian` already relies on — so no
//! precomputed range list or thread-local state is needed to reach a call
//! inside a reexpanded function body.

use smelt_parser::ast::FunctionCall;
use smelt_parser::syntax_kind::{SyntaxKind, SyntaxNode};
use smelt_types::{BuiltinRegistry, CallFacts, RewriteId, SettledEmission};

use crate::position::classify as classify_position;
use crate::SqlDialect;

/// Should `frame` — a `WINDOW_FRAME` node — be dropped when printing for
/// `dialect`?
///
/// `false` for a `WINDOW_FRAME` reached through a named window (`OVER w`,
/// with `w`'s frame declared on a shared `WINDOW w AS (...)` clause): the
/// frame there is not the call's own sibling node (it hangs off a
/// `NAMED_WINDOW`, not a `WINDOW_SPEC`), and dropping it would silently
/// change every window that shares the name. The frame reaches the target
/// dialect unchanged instead, so a backend refusal (if any) fires loud on
/// the engine rather than being silently elided.
pub(crate) fn should_elide(frame: &SyntaxNode, dialect: &SqlDialect) -> bool {
    debug_assert_eq!(frame.kind(), SyntaxKind::WINDOW_FRAME);
    let Some(spec) = frame.parent() else {
        return false;
    };
    if spec.kind() != SyntaxKind::WINDOW_SPEC {
        return false;
    }
    // The grammar places `FUNCTION_CALL` and its optional `WINDOW_SPEC` as
    // sibling children of the enclosing `EXPRESSION` (mirrors
    // `position::window_spec_sibling`, in the opposite direction).
    let Some(parent) = spec.parent() else {
        return false;
    };
    let Some(call) = parent
        .children()
        .find(|n| n.kind() == SyntaxKind::FUNCTION_CALL)
    else {
        return false;
    };
    let Some(fc) = FunctionCall::cast(call.clone()) else {
        return false;
    };
    let Some(name) = fc.name() else {
        return false;
    };
    let Some(sig) = BuiltinRegistry::resolve(&name) else {
        return false;
    };
    let root = call.ancestors().last().unwrap_or_else(|| call.clone());
    let position = classify_position(&call, &root);
    let arity = fc.arguments().len();
    matches!(
        sig.settle_at(dialect.id(), position, &CallFacts::unresolved(arity)),
        SettledEmission::Rewrite(RewriteId::ElideWindowFrame)
    )
}
