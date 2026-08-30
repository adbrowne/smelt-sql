//! Tests for `smelt_dialect::position::classify` — the pure classifier that
//! decides a `FUNCTION_CALL`'s SQL call position from its source CST.
//!
//! Correctness oracle: `docs/specs/multi_backend.md` §"Emission is scoped to
//! call position".

use smelt_dialect::position::classify;
use smelt_parser::parser::parse;
use smelt_parser::syntax_kind::{SyntaxKind, SyntaxNode};
use smelt_types::signatures::Position;

/// Parse `sql` and return every `FUNCTION_CALL` node in source order.
fn function_calls(root: &SyntaxNode) -> Vec<SyntaxNode> {
    root.descendants()
        .filter(|n| n.kind() == SyntaxKind::FUNCTION_CALL)
        .collect()
}

/// Parse `sql`, assert it has no parse errors, and return the root node.
fn parse_clean(sql: &str) -> SyntaxNode {
    let parsed = parse(sql);
    assert!(
        parsed.errors.is_empty(),
        "unexpected parse errors for {sql:?}: {:?}",
        parsed.errors
    );
    parsed.syntax()
}

#[test]
fn scalar_call_is_scalar() {
    let root = parse_clean("SELECT UPPER(name) AS n FROM t");
    let calls = function_calls(&root);
    assert_eq!(calls.len(), 1);
    assert_eq!(classify(&calls[0], &root), Position::Scalar);
}

#[test]
fn aggregate_under_group_by_is_aggregate() {
    let root = parse_clean("SELECT g, SUM(x) AS s FROM t GROUP BY g");
    let calls = function_calls(&root);
    assert_eq!(calls.len(), 1);
    assert_eq!(classify(&calls[0], &root), Position::Aggregate);
}

#[test]
fn scalar_call_on_grouping_key_in_select_list_is_scalar() {
    // UPPER(g) is a plain scalar call on the grouping key, not an aggregate —
    // the mere presence of a GROUP BY on the enclosing statement must not
    // taint an unrelated scalar call in the projection.
    let root = parse_clean("SELECT g, UPPER(g) AS u FROM t GROUP BY g");
    let calls = function_calls(&root);
    assert_eq!(calls.len(), 1);
    assert_eq!(classify(&calls[0], &root), Position::Scalar);
}

#[test]
fn scalar_call_in_where_clause_with_group_by_is_scalar() {
    // UPPER(name) sits in WHERE, where aggregates are not even legal — an
    // enclosing GROUP BY on the statement must not affect it.
    let root = parse_clean("SELECT g, COUNT(*) AS c FROM t WHERE UPPER(name) = 'X' GROUP BY g");
    let calls = function_calls(&root);
    assert_eq!(calls.len(), 2);
    let where_call = calls
        .iter()
        .find(|c| {
            smelt_parser::ast::FunctionCall::cast((*c).clone())
                .and_then(|fc| fc.name())
                .as_deref()
                == Some("UPPER")
        })
        .expect("UPPER call present");
    assert_eq!(classify(where_call, &root), Position::Scalar);
}

#[test]
fn aggregate_call_with_no_group_by_is_still_aggregate() {
    // COUNT(*) is an implicit single-group aggregate; there is no GROUP BY
    // clause at all on the statement.
    let root = parse_clean("SELECT COUNT(*) AS c FROM t");
    let calls = function_calls(&root);
    assert_eq!(calls.len(), 1);
    assert_eq!(classify(&calls[0], &root), Position::Aggregate);
}

#[test]
fn partition_only_window_is_whole_partition() {
    let root = parse_clean("SELECT MEDIAN(x) OVER (PARTITION BY g) AS m FROM t");
    let calls = function_calls(&root);
    assert_eq!(calls.len(), 1);
    assert_eq!(classify(&calls[0], &root), Position::WholePartitionWindow);
}

#[test]
fn explicit_unbounded_frame_is_whole_partition() {
    let root = parse_clean(
        "SELECT SUM(x) OVER (PARTITION BY g ROWS BETWEEN UNBOUNDED PRECEDING AND UNBOUNDED FOLLOWING) AS s FROM t",
    );
    let calls = function_calls(&root);
    assert_eq!(calls.len(), 1);
    assert_eq!(classify(&calls[0], &root), Position::WholePartitionWindow);
}

#[test]
fn order_by_without_frame_is_running() {
    let root = parse_clean("SELECT SUM(x) OVER (PARTITION BY g ORDER BY t) AS s FROM t");
    let calls = function_calls(&root);
    assert_eq!(calls.len(), 1);
    assert_eq!(classify(&calls[0], &root), Position::Window);
}

#[test]
fn exclude_clause_defeats_whole_partition() {
    let root = parse_clean(
        "SELECT SUM(x) OVER (PARTITION BY g ROWS BETWEEN UNBOUNDED PRECEDING AND UNBOUNDED FOLLOWING EXCLUDE CURRENT ROW) AS s FROM t",
    );
    let calls = function_calls(&root);
    assert_eq!(calls.len(), 1);
    assert_eq!(classify(&calls[0], &root), Position::Window);
}

#[test]
fn named_window_is_resolved_before_classifying() {
    let root =
        parse_clean("SELECT SUM(x) OVER w AS s FROM t WINDOW w AS (PARTITION BY g ORDER BY t)");
    let calls = function_calls(&root);
    assert_eq!(calls.len(), 1);
    // The window's own site carries no ORDER BY or frame; only resolving
    // `w` reveals the ORDER BY that makes this a running window.
    assert_eq!(classify(&calls[0], &root), Position::Window);
}

#[test]
fn unresolvable_named_window_is_running() {
    // No WINDOW clause defines `w` at all — refusing is the safe direction.
    let root = parse_clean("SELECT SUM(x) OVER w AS s FROM t");
    let calls = function_calls(&root);
    assert_eq!(calls.len(), 1);
    assert_eq!(classify(&calls[0], &root), Position::Window);

    // An inline window spec that error-recovery leaves without a closing
    // paren (an "inheriting" `OVER (w ORDER BY t)` form this grammar does
    // not support standalone) must not be mistaken for a clean, whole-
    // partition `OVER ()` — the classifier must still resolve to `Window`
    // rather than panicking or guessing whole-partition.
    let parsed =
        parse("SELECT sum(x) OVER (w ORDER BY t) AS a FROM t WINDOW w AS (PARTITION BY g)");
    assert!(
        !parsed.errors.is_empty(),
        "this construct is expected to be a parse error under the current grammar"
    );
    let root = parsed.syntax();
    let calls = function_calls(&root);
    assert!(!calls.is_empty());
    assert_eq!(classify(&calls[0], &root), Position::Window);
}

#[test]
fn classifies_every_call_in_example_model() {
    let sql = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/retail_analytics/models/intermediate/int_customer_orders.sql"
    ))
    .expect("fixture model should exist");
    let root = parse_clean(&sql);
    let calls = function_calls(&root);
    assert!(!calls.is_empty(), "fixture model should contain calls");
    for call in &calls {
        // Total: every FUNCTION_CALL in real source gets *some* position,
        // and it is never the lookup-only wildcard.
        let position = classify(call, &root);
        assert_ne!(position, Position::Any, "classify must never return Any");
    }
}
