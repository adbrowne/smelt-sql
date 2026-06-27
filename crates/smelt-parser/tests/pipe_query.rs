//! Phase 1 TDD tests for the pipe SQL (`|>`) grammar spine.
//!
//! These tests cover:
//! 1. FROM-first trigger
//! 2. WITH + FROM-first trigger
//! 3. SELECT body is NOT a pipe query
//! 4. Every supported operator parses to a tagged PIPE_STAGE
//! 5. Unknown operator diagnostic
//! 6. Malformed stage diagnostic
//! 7. Deferred operator diagnostic

use smelt_parser::parse;
use smelt_parser::syntax_kind::SyntaxKind;

/// Helper: collect all node kinds in the CST (breadth-first).
#[allow(dead_code)]
fn all_kinds(node: &smelt_parser::syntax_kind::SyntaxNode) -> Vec<SyntaxKind> {
    let mut result = Vec::new();
    let mut queue = std::collections::VecDeque::new();
    queue.push_back(node.clone());
    while let Some(n) = queue.pop_front() {
        result.push(n.kind());
        for child in n.children() {
            queue.push_back(child);
        }
    }
    result
}

/// Helper: find a node of the given kind in the CST (depth-first pre-order).
fn find_node(
    node: &smelt_parser::syntax_kind::SyntaxNode,
    kind: SyntaxKind,
) -> Option<smelt_parser::syntax_kind::SyntaxNode> {
    if node.kind() == kind {
        return Some(node.clone());
    }
    for child in node.children() {
        if let Some(found) = find_node(&child, kind) {
            return Some(found);
        }
    }
    None
}

/// Helper: count nodes of the given kind in the CST.
#[allow(dead_code)]
fn count_nodes(node: &smelt_parser::syntax_kind::SyntaxNode, kind: SyntaxKind) -> usize {
    let mut count = if node.kind() == kind { 1 } else { 0 };
    for child in node.children() {
        count += count_nodes(&child, kind);
    }
    count
}

/// Helper: find all nodes of the given kind (depth-first pre-order).
fn find_all_nodes(
    node: &smelt_parser::syntax_kind::SyntaxNode,
    kind: SyntaxKind,
) -> Vec<smelt_parser::syntax_kind::SyntaxNode> {
    let mut result = Vec::new();
    if node.kind() == kind {
        result.push(node.clone());
    }
    for child in node.children() {
        result.extend(find_all_nodes(&child, kind));
    }
    result
}

// ── Test 1: from_first_triggers_pipe_query ──────────────────────────────────

#[test]
fn from_first_triggers_pipe_query() {
    let input = "FROM orders |> WHERE status = 'paid'";
    let p = parse(input);
    assert!(
        p.errors.is_empty(),
        "expected no parse errors, got: {:?}",
        p.errors
    );

    let root = p.syntax();
    // The FILE root must contain a PIPE_QUERY node.
    let pipe_query = find_node(&root, SyntaxKind::PIPE_QUERY).expect("expected PIPE_QUERY node");

    // PIPE_QUERY must contain a FROM_CLAUSE.
    let from_clause = find_node(&pipe_query, SyntaxKind::FROM_CLAUSE);
    assert!(
        from_clause.is_some(),
        "expected FROM_CLAUSE inside PIPE_QUERY"
    );

    // PIPE_QUERY must contain exactly one PIPE_STAGE.
    let stages = find_all_nodes(&pipe_query, SyntaxKind::PIPE_STAGE);
    assert_eq!(stages.len(), 1, "expected exactly one PIPE_STAGE");

    // The PIPE_STAGE must contain a PIPE_OP_WHERE marker.
    let stage = &stages[0];
    let where_marker = find_node(stage, SyntaxKind::PIPE_OP_WHERE);
    assert!(
        where_marker.is_some(),
        "expected PIPE_OP_WHERE marker in WHERE stage"
    );
}

// ── Test 2: leading_with_then_from_first ────────────────────────────────────

#[test]
fn leading_with_then_from_first() {
    let input = "WITH r AS (SELECT 1) FROM r |> LIMIT 1";
    let p = parse(input);
    assert!(
        p.errors.is_empty(),
        "expected no parse errors, got: {:?}",
        p.errors
    );

    let root = p.syntax();
    // Must produce a PIPE_QUERY (not a SELECT_STMT).
    let pipe_query = find_node(&root, SyntaxKind::PIPE_QUERY)
        .expect("expected PIPE_QUERY for WITH + FROM-first");

    // The PIPE_QUERY must contain a WITH_CLAUSE.
    let with_clause = find_node(&pipe_query, SyntaxKind::WITH_CLAUSE);
    assert!(
        with_clause.is_some(),
        "expected WITH_CLAUSE inside PIPE_QUERY"
    );

    // The FILE's direct children must NOT include a SELECT_STMT —
    // the model body is a PIPE_QUERY, not a SELECT_STMT. (The CTE body
    // `SELECT 1` inside `WITH r AS (SELECT 1)` will produce a SELECT_STMT
    // inside the CTE, but NOT as a direct child of FILE.)
    let file_level_select = root.children().any(|c| c.kind() == SyntaxKind::SELECT_STMT);
    assert!(
        !file_level_select,
        "SELECT_STMT should not appear as a direct FILE child for a WITH + FROM-first body"
    );

    // Must have one PIPE_STAGE (LIMIT).
    let stages = find_all_nodes(&pipe_query, SyntaxKind::PIPE_STAGE);
    assert_eq!(stages.len(), 1, "expected one PIPE_STAGE");
    let limit_marker = find_node(&stages[0], SyntaxKind::PIPE_OP_LIMIT);
    assert!(limit_marker.is_some(), "expected PIPE_OP_LIMIT marker");
}

// ── Test 3: select_body_is_not_pipe ─────────────────────────────────────────

#[test]
fn select_body_is_not_pipe() {
    let input = "SELECT 1 FROM t";
    let p = parse(input);
    // No parse errors.
    assert!(
        p.errors.is_empty(),
        "expected no parse errors, got: {:?}",
        p.errors
    );

    let root = p.syntax();
    // Must produce a SELECT_STMT, not a PIPE_QUERY.
    let select_stmt = find_node(&root, SyntaxKind::SELECT_STMT);
    assert!(
        select_stmt.is_some(),
        "expected SELECT_STMT for SELECT-first body"
    );

    let pipe_query = find_node(&root, SyntaxKind::PIPE_QUERY);
    assert!(
        pipe_query.is_none(),
        "SELECT-first body must not produce PIPE_QUERY"
    );
}

// ── Test 4: each_supported_operator_parses ───────────────────────────────────

#[test]
fn each_supported_operator_parses() {
    // Each case: (SQL input, expected stage-kind marker)
    let cases: &[(&str, SyntaxKind)] = &[
        ("FROM t |> WHERE x = 1", SyntaxKind::PIPE_OP_WHERE),
        ("FROM t |> SELECT a, b", SyntaxKind::PIPE_OP_SELECT),
        ("FROM t |> EXTEND x + 1 AS y", SyntaxKind::PIPE_OP_EXTEND),
        ("FROM t |> SET a = 1", SyntaxKind::PIPE_OP_SET),
        ("FROM t |> DROP col1", SyntaxKind::PIPE_OP_DROP),
        ("FROM t |> RENAME old AS new", SyntaxKind::PIPE_OP_RENAME),
        ("FROM t |> AS my_alias", SyntaxKind::PIPE_OP_AS),
        (
            "FROM t |> AGGREGATE count(*) AS n",
            SyntaxKind::PIPE_OP_AGGREGATE,
        ),
        ("FROM t |> ORDER BY col", SyntaxKind::PIPE_OP_ORDER_BY),
        ("FROM t |> LIMIT 10", SyntaxKind::PIPE_OP_LIMIT),
        ("FROM t |> JOIN s ON t.id = s.id", SyntaxKind::PIPE_OP_JOIN),
        (
            "FROM t |> UNION ALL (SELECT * FROM u)",
            SyntaxKind::PIPE_OP_UNION,
        ),
        (
            "FROM t |> INTERSECT ALL (SELECT * FROM u)",
            SyntaxKind::PIPE_OP_INTERSECT,
        ),
        (
            "FROM t |> EXCEPT ALL (SELECT * FROM u)",
            SyntaxKind::PIPE_OP_EXCEPT,
        ),
        ("FROM t |> DISTINCT", SyntaxKind::PIPE_OP_DISTINCT),
    ];

    for (sql, expected_marker) in cases {
        let p = parse(sql);
        assert!(
            p.errors.is_empty(),
            "operator {:?}: unexpected parse errors: {:?}",
            expected_marker,
            p.errors
        );

        let root = p.syntax();
        let pipe_query = find_node(&root, SyntaxKind::PIPE_QUERY).unwrap_or_else(|| {
            panic!(
                "operator {:?}: no PIPE_QUERY node in: {}",
                expected_marker, sql
            )
        });

        let stages = find_all_nodes(&pipe_query, SyntaxKind::PIPE_STAGE);
        assert_eq!(
            stages.len(),
            1,
            "operator {:?}: expected 1 PIPE_STAGE, got {}",
            expected_marker,
            stages.len()
        );

        let stage = &stages[0];
        let marker = find_node(stage, *expected_marker);
        assert!(
            marker.is_some(),
            "operator {:?}: expected marker {:?} in stage for: {}",
            expected_marker,
            expected_marker,
            sql
        );
    }
}

// ── Test 5: unknown_operator_diagnostic ─────────────────────────────────────

#[test]
fn unknown_operator_diagnostic() {
    let input = "FROM t |> FROBNICATE x";
    let p = parse(input);

    // Must emit exactly one error about the unknown operator.
    let unknown_errors: Vec<_> = p
        .errors
        .iter()
        .filter(|e| e.message.contains("unknown pipe operator"))
        .collect();
    assert_eq!(
        unknown_errors.len(),
        1,
        "expected one 'unknown pipe operator' error, got: {:?}",
        p.errors
    );
    // The error message should mention the keyword.
    assert!(
        unknown_errors[0].message.contains("FROBNICATE"),
        "error message should mention the unknown keyword, got: {}",
        unknown_errors[0].message
    );
}

// ── Test 6: malformed_stage_diagnostic ──────────────────────────────────────

#[test]
fn malformed_stage_diagnostic() {
    // WHERE with no predicate is malformed.
    let input = "FROM t |> WHERE";
    let p = parse(input);

    // Must emit at least one error.
    assert!(
        !p.errors.is_empty(),
        "expected parse errors for malformed WHERE stage"
    );

    // Should contain a PipeStageMalformed-style message OR a generic parse error.
    // Phase 1: the parser emits a plain error (not yet a smelt-db semantic diagnostic).
    // The error message should mention "WHERE" or "malformed".
    let has_relevant_error = p.errors.iter().any(|e| {
        e.message.to_lowercase().contains("where")
            || e.message.to_lowercase().contains("malformed")
            || e.message.to_lowercase().contains("expected")
    });
    assert!(
        has_relevant_error,
        "expected a relevant error for malformed WHERE stage, got: {:?}",
        p.errors
    );

    // The CST should still have a PIPE_QUERY (error recovery).
    let root = p.syntax();
    let pipe_query = find_node(&root, SyntaxKind::PIPE_QUERY);
    assert!(
        pipe_query.is_some(),
        "expected PIPE_QUERY even after malformed stage (error recovery)"
    );
}

// ── Test 8: join_variants_parse ─────────────────────────────────────────────

#[test]
fn join_variants_parse() {
    let cases = &[
        ("FROM t |> JOIN s ON t.id = s.id", SyntaxKind::PIPE_OP_JOIN),
        (
            "FROM t |> INNER JOIN s ON t.id = s.id",
            SyntaxKind::PIPE_OP_JOIN,
        ),
        (
            "FROM t |> LEFT JOIN s ON t.id = s.id",
            SyntaxKind::PIPE_OP_JOIN,
        ),
        (
            "FROM t |> RIGHT JOIN s ON t.id = s.id",
            SyntaxKind::PIPE_OP_JOIN,
        ),
        (
            "FROM t |> FULL JOIN s ON t.id = s.id",
            SyntaxKind::PIPE_OP_JOIN,
        ),
        ("FROM t |> CROSS JOIN s", SyntaxKind::PIPE_OP_JOIN),
        ("FROM t |> JOIN s USING (id)", SyntaxKind::PIPE_OP_JOIN),
    ];
    for (sql, expected_marker) in cases {
        let p = parse(sql);
        assert!(
            p.errors.is_empty(),
            "join variant parse error for `{sql}`: {:?}",
            p.errors
        );
        let root = p.syntax();
        let pq = find_node(&root, SyntaxKind::PIPE_QUERY)
            .unwrap_or_else(|| panic!("no PIPE_QUERY for: {sql}"));
        let stages = find_all_nodes(&pq, SyntaxKind::PIPE_STAGE);
        assert_eq!(stages.len(), 1, "expected 1 PIPE_STAGE for: {sql}");
        let join_marker = find_node(&stages[0], *expected_marker);
        assert!(
            join_marker.is_some(),
            "expected {:?} marker for: {sql}",
            expected_marker
        );
    }
}

// ── Test 7: deferred_operator_diagnostic ────────────────────────────────────

#[test]
fn deferred_operator_diagnostic() {
    let input = "FROM t |> PIVOT(count(*) FOR status IN ('a', 'b'))";
    let p = parse(input);

    // Must emit at least one error about the unsupported/deferred operator.
    let unsupported_errors: Vec<_> = p
        .errors
        .iter()
        .filter(|e| {
            e.message.contains("not supported")
                || e.message.contains("unsupported")
                || e.message.contains("PipeOperatorUnsupported")
        })
        .collect();
    assert_eq!(
        unsupported_errors.len(),
        1,
        "expected one 'not supported' error for PIVOT, got: {:?}",
        p.errors
    );
    // Message should mention PIVOT.
    assert!(
        unsupported_errors[0].message.contains("PIVOT"),
        "error message should mention PIVOT, got: {}",
        unsupported_errors[0].message
    );

    // The CST should still have a PIPE_QUERY (error recovery).
    let root = p.syntax();
    let pipe_query = find_node(&root, SyntaxKind::PIPE_QUERY);
    assert!(
        pipe_query.is_some(),
        "expected PIPE_QUERY even after deferred operator (error recovery)"
    );
}
