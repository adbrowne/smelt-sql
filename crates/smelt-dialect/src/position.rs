//! Classifies a call's SQL position from its source CST.
//!
//! A built-in's emission verdict is stated per `(dialect, position)`, never
//! per dialect alone, because a backend's support for a built-in routinely
//! differs between the positions it can appear in. Position is decided once,
//! here, from the source CST, and handed to the registry — no other consumer
//! (in particular, the printer) re-derives it.
//!
//! Correctness oracle: `docs/specs/multi_backend.md` §"Emission is scoped to
//! call position".

use smelt_parser::ast::{FunctionCall, NamedWindow, WindowClause, WindowSpec};
use smelt_parser::syntax_kind::{SyntaxKind, SyntaxNode};
use smelt_types::signatures::{ExprKind, Position};
use smelt_types::BuiltinRegistry;

/// Classify `node` — a `FUNCTION_CALL` — by the SQL position it occupies.
///
/// `root` is searched to resolve a named-window reference (`OVER w`) when
/// the call's own statement does not carry a matching `WINDOW` clause entry
/// in an ancestor scope reachable from `node`; passing the enclosing `FILE`
/// (or any ancestor of `node`) is sufficient.
///
/// Pure: no I/O. Never returns [`Position::Any`] — that variant exists only
/// as a lookup wildcard for registry entries whose verdict does not vary by
/// position.
pub fn classify(node: &SyntaxNode, root: &SyntaxNode) -> Position {
    debug_assert_eq!(
        node.kind(),
        SyntaxKind::FUNCTION_CALL,
        "position::classify expects a FUNCTION_CALL node"
    );

    match window_spec_sibling(node) {
        Some(spec) => classify_window(&spec, root),
        None => classify_non_window(node),
    }
}

/// The `WINDOW_SPEC` node attached to `node`'s `OVER` clause, if any. The
/// grammar places `FUNCTION_CALL` and its optional `WINDOW_SPEC` as sibling
/// children of the enclosing `EXPRESSION`.
fn window_spec_sibling(node: &SyntaxNode) -> Option<SyntaxNode> {
    let parent = node.parent()?;
    parent
        .children()
        .find(|n| n.kind() == SyntaxKind::WINDOW_SPEC)
}

/// Nearest enclosing `SELECT_STMT`, walking up from `node`. Scoping resolves
/// through this rather than through `root` directly, so a nested subquery's
/// own `GROUP BY` / `WINDOW` clause is never confused with an outer one.
fn enclosing_select_stmt(node: &SyntaxNode) -> Option<SyntaxNode> {
    node.ancestors()
        .find(|n| n.kind() == SyntaxKind::SELECT_STMT)
}

// ─── Non-window calls: Scalar / Aggregate ──────────────────────────────────

fn classify_non_window(node: &SyntaxNode) -> Position {
    if is_registry_aggregate(node) {
        Position::Aggregate
    } else {
        Position::Scalar
    }
}

/// True when the call's own name resolves in the registry as an aggregate —
/// covers `SELECT COUNT(*) FROM t`, an aggregate with no `OVER` and no
/// enclosing `GROUP BY` (an implicit single-group aggregate).
fn is_registry_aggregate(node: &SyntaxNode) -> bool {
    FunctionCall::cast(node.clone())
        .and_then(|fc| fc.name())
        .and_then(|name| BuiltinRegistry::resolve(&name))
        .map(|sig| sig.kind == ExprKind::Agg)
        .unwrap_or(false)
}

// ─── Window calls: WholePartitionWindow / Window ───────────────────────────

fn classify_window(spec: &SyntaxNode, root: &SyntaxNode) -> Position {
    // A `WINDOW_SPEC` that error-recovery left without a closing paren (or,
    // for the bare-name form, without the name token) is not a clean spec at
    // all. Guessing whole-partition on malformed input would be exactly the
    // silent-wrong-answer this classifier exists to avoid — refuse to the
    // safe direction instead.
    if !is_well_formed(spec) {
        return Position::Window;
    }

    let Some(window_spec) = WindowSpec::cast(spec.clone()) else {
        return Position::Window;
    };

    match window_spec.window_name() {
        Some(name) => match resolve_named_window(&name, spec, root) {
            Some(named) => shape_of(named.syntax()),
            // Refusing is the safe direction: it costs a diagnostic, where
            // guessing whole-partition costs a wrong number.
            None => Position::Window,
        },
        None => shape_of(spec),
    }
}

/// A `WINDOW_SPEC` is well-formed if its parenthesized form is properly
/// closed, or its bare-name form (`OVER w`) actually carries the name.
/// Anything else is the product of parse-error recovery.
fn is_well_formed(spec: &SyntaxNode) -> bool {
    let mut has_lparen = false;
    let mut has_rparen = false;
    let mut has_ident = false;
    for token in spec.children_with_tokens().filter_map(|e| e.into_token()) {
        match token.kind() {
            SyntaxKind::LPAREN => has_lparen = true,
            SyntaxKind::RPAREN => has_rparen = true,
            SyntaxKind::IDENT => has_ident = true,
            _ => {}
        }
    }
    if has_lparen {
        has_rparen
    } else {
        has_ident
    }
}

/// Resolve a named-window reference to its definition. Prefers the `WINDOW`
/// clause of the call's own nearest enclosing `SELECT_STMT` (correct SQL
/// scoping — named windows are not inherited across statement boundaries);
/// falls back to searching all of `root` for robustness when `spec` has no
/// such ancestor reachable (e.g. a standalone spec in a test fixture).
fn resolve_named_window(name: &str, spec: &SyntaxNode, root: &SyntaxNode) -> Option<NamedWindow> {
    if let Some(select) = enclosing_select_stmt(spec) {
        if let Some(found) = find_named_window(&select, name) {
            return Some(found);
        }
    }
    find_named_window(root, name)
}

fn find_named_window(scope: &SyntaxNode, name: &str) -> Option<NamedWindow> {
    scope
        .descendants()
        .filter(|n| n.kind() == SyntaxKind::WINDOW_CLAUSE)
        .filter_map(WindowClause::cast)
        .flat_map(|wc| wc.named_windows().collect::<Vec<_>>())
        .find(|nw| nw.name().as_deref() == Some(name))
}

/// Whole-partition vs running, for a container (`WINDOW_SPEC` or
/// `NAMED_WINDOW`) that directly holds an optional `ORDER_BY_CLAUSE` and an
/// optional `WINDOW_FRAME` child — both node shapes share this layout.
fn shape_of(container: &SyntaxNode) -> Position {
    let has_order_by = container
        .children()
        .any(|n| n.kind() == SyntaxKind::ORDER_BY_CLAUSE);
    let frame = container
        .children()
        .find(|n| n.kind() == SyntaxKind::WINDOW_FRAME);

    match frame {
        Some(frame) => {
            if frame_is_whole_partition(&frame) {
                Position::WholePartitionWindow
            } else {
                Position::Window
            }
        }
        None => {
            if has_order_by {
                // The SQL default frame is `RANGE BETWEEN UNBOUNDED
                // PRECEDING AND CURRENT ROW` — running.
                Position::Window
            } else {
                Position::WholePartitionWindow
            }
        }
    }
}

/// `BETWEEN UNBOUNDED PRECEDING AND UNBOUNDED FOLLOWING` with no `EXCLUDE`
/// clause. `EXCLUDE` changes the answer per row even under this exact frame
/// wording (measured: DuckDB's `SUM` over `1,2,3` returns `5, 4, 3` under
/// `... EXCLUDE CURRENT ROW`), so its presence always defeats whole-partition
/// classification regardless of the bounds.
fn frame_is_whole_partition(frame: &SyntaxNode) -> bool {
    let has_exclude = frame
        .children()
        .any(|n| n.kind() == SyntaxKind::FRAME_EXCLUDE);
    if has_exclude {
        return false;
    }

    let bounds: Vec<SyntaxNode> = frame
        .children()
        .filter(|n| n.kind() == SyntaxKind::FRAME_BOUND)
        .collect();
    let [start, end] = bounds.as_slice() else {
        // A single bound implies `AND CURRENT ROW` as the end — never
        // whole-partition.
        return false;
    };
    is_unbounded(start, SyntaxKind::PRECEDING_KW) && is_unbounded(end, SyntaxKind::FOLLOWING_KW)
}

fn is_unbounded(bound: &SyntaxNode, direction: SyntaxKind) -> bool {
    let tokens: Vec<SyntaxKind> = bound
        .children_with_tokens()
        .filter_map(|e| e.into_token())
        .map(|t| t.kind())
        .filter(|k| !matches!(k, SyntaxKind::WHITESPACE | SyntaxKind::COMMENT))
        .collect();
    tokens == [SyntaxKind::UNBOUNDED_KW, direction]
}
