//! `definition_diff`: the CST-level factoring of a (before, after) pair of
//! model definitions into a [`DefinitionDiff`]. See the module doc comment
//! in `backbuild/mod.rs` and `docs/research/20260802-backbuild-synthesis.md`
//! §3 ("Inputs and outputs") and §4 ("The catalogue").
//!
//! Purely syntactic: every comparator below is trivia-insensitive structural
//! equality over already-parsed CST nodes, never a scan over raw SQL text.
//! Fail-closed throughout — any shape a comparator does not recognise
//! reports `Opaque`/`Changed` with a reason, never a silent "unchanged".

use super::{
    ChangedColumn, ComparableDiff, ConjunctDiff, DefinitionDiff, EditedBranch, SelectColumn,
    SelectListDiff, SetOpDiff, SkeletonDiff,
};
use smelt_parser::syntax_kind::SyntaxNode;
use smelt_parser::{Expr, File, JoinClause, JoinType, SelectEntry, SelectList, SelectStmt};

/// Factor `before` and `after` into a [`DefinitionDiff`].
pub fn definition_diff(before: &File, after: &File) -> DefinitionDiff {
    let (before_stmt, after_stmt) = match (before.select_stmt(), after.select_stmt()) {
        (Some(b), Some(a)) => (b, a),
        _ => {
            return DefinitionDiff::Opaque {
                reason: "one or both definitions are not a plain SELECT statement".to_string(),
            }
        }
    };

    let before_with = before_stmt.with_clause();
    let after_with = after_stmt.with_clause();
    let cte_unchanged = match (&before_with, &after_with) {
        (None, None) => true,
        (Some(b), Some(a)) => same_modulo_trivia(b.syntax(), a.syntax()),
        _ => false,
    };
    if !cte_unchanged {
        return DefinitionDiff::Opaque {
            reason: "the WITH-prefix (CTE section) changed between versions; the pure diff \
                     module refuses to chase CTE lineage"
                .to_string(),
        };
    }

    let select_list = select_list_diff(
        before_stmt.select_list().as_ref(),
        after_stmt.select_list().as_ref(),
    );
    let where_clause = where_diff(&before_stmt, &after_stmt);
    let skeleton = skeleton_diff(&before_stmt, &after_stmt);
    let set_ops = set_op_diff(&before_stmt, &after_stmt);

    DefinitionDiff::Comparable(Box::new(ComparableDiff {
        select_list,
        where_clause,
        skeleton,
        set_ops,
    }))
}

/// Trivia-insensitive structural equality: two syntax subtrees are equal
/// when their non-trivia token kind+text sequences match. Whitespace and
/// comments are skipped; token text is compared exactly (case-preserving —
/// a case *change* is not a no-op). Used by every clause comparator below.
fn same_modulo_trivia(a: &SyntaxNode, b: &SyntaxNode) -> bool {
    let mut ta = a
        .descendants_with_tokens()
        .filter_map(|e| e.into_token())
        .filter(|t| !t.kind().is_trivia())
        .map(|t| (t.kind(), t.text().to_string()));
    let mut tb = b
        .descendants_with_tokens()
        .filter_map(|e| e.into_token())
        .filter(|t| !t.kind().is_trivia())
        .map(|t| (t.kind(), t.text().to_string()));
    loop {
        match (ta.next(), tb.next()) {
            (None, None) => return true,
            (Some(x), Some(y)) if x == y => continue,
            _ => return false,
        }
    }
}

// ===== SELECT list =====

fn select_list_diff(before: Option<&SelectList>, after: Option<&SelectList>) -> SelectListDiff {
    let (before, after) = match (before, after) {
        (Some(b), Some(a)) => (b, a),
        _ => {
            return SelectListDiff::Opaque {
                reason: "one or both definitions have no SELECT list".to_string(),
            }
        }
    };

    let before_cols = match collect_named_columns(before) {
        Ok(cols) => cols,
        Err(reason) => {
            return SelectListDiff::Opaque {
                reason: format!("before: {reason}"),
            }
        }
    };
    let after_cols = match collect_named_columns(after) {
        Ok(cols) => cols,
        Err(reason) => {
            return SelectListDiff::Opaque {
                reason: format!("after: {reason}"),
            }
        }
    };

    if let Some(dup) = first_duplicate_name(&before_cols) {
        return SelectListDiff::Opaque {
            reason: format!(
                "before SELECT list has duplicate output column name '{dup}' — ambiguous to key by name"
            ),
        };
    }
    if let Some(dup) = first_duplicate_name(&after_cols) {
        return SelectListDiff::Opaque {
            reason: format!(
                "after SELECT list has duplicate output column name '{dup}' — ambiguous to key by name"
            ),
        };
    }

    let before_order: Vec<String> = before_cols.iter().map(|c| c.name.clone()).collect();
    let after_order: Vec<String> = after_cols.iter().map(|c| c.name.clone()).collect();

    let mut dropped = Vec::new();
    let mut changed = Vec::new();
    let mut unchanged = Vec::new();

    for bc in &before_cols {
        match after_cols.iter().find(|ac| ac.name == bc.name) {
            None => dropped.push(bc.clone()),
            Some(ac) => {
                if same_modulo_trivia(bc.expr.syntax(), ac.expr.syntax()) {
                    unchanged.push(bc.clone());
                } else {
                    changed.push(ChangedColumn {
                        name: bc.name.clone(),
                        before: bc.expr.clone(),
                        after: ac.expr.clone(),
                    });
                }
            }
        }
    }

    let added = after_cols
        .into_iter()
        .filter(|ac| !before_cols.iter().any(|bc| bc.name == ac.name))
        .collect();

    SelectListDiff::Diffed {
        added,
        dropped,
        changed,
        unchanged,
        before_order,
        after_order,
    }
}

/// Resolve every entry of `list` to a named `(column name, expression)`
/// pair. Fails closed (rather than silently skipping) on any entry this
/// module cannot key by output name: a wildcard (`*`/`qualifier.*`), a
/// spread (`...expr`), an item with no derivable name, or an item with no
/// expression.
fn collect_named_columns(list: &SelectList) -> Result<Vec<SelectColumn>, String> {
    let mut out = Vec::new();
    for entry in list.entries() {
        match entry {
            SelectEntry::Item(item) => {
                if item.is_wildcard() {
                    return Err("a wildcard SELECT item (`*`) is not resolvable to a named \
                                 column by the pure diff"
                        .to_string());
                }
                let name = item.column_name().ok_or_else(|| {
                    "a SELECT item has no derivable output column name".to_string()
                })?;
                let expr = item
                    .expression()
                    .ok_or_else(|| format!("SELECT item '{name}' has no expression"))?;
                out.push(SelectColumn { name, expr });
            }
            SelectEntry::Spread(_) => {
                return Err(
                    "a spread (`...expr`) SELECT item is not resolvable to a named column by \
                     the pure diff"
                        .to_string(),
                );
            }
        }
    }
    Ok(out)
}

fn first_duplicate_name(cols: &[SelectColumn]) -> Option<String> {
    let mut seen = std::collections::HashSet::new();
    for c in cols {
        if !seen.insert(c.name.as_str()) {
            return Some(c.name.clone());
        }
    }
    None
}

// ===== WHERE conjunct set =====

fn where_diff(before: &SelectStmt, after: &SelectStmt) -> ConjunctDiff {
    let before_clause = before.where_clause();
    let after_clause = after.where_clause();

    let before_expr = match &before_clause {
        Some(w) => match w.expression() {
            Some(e) => Some(e),
            None => {
                // Malformed-clause degenerate case (no expression at all) —
                // nothing meaningful to retain for a later alias sweep.
                return ConjunctDiff::Opaque {
                    reason: "before WHERE clause has no expression".to_string(),
                    before: None,
                    after: None,
                };
            }
        },
        None => None,
    };
    let after_expr = match &after_clause {
        Some(w) => match w.expression() {
            Some(e) => Some(e),
            None => {
                return ConjunctDiff::Opaque {
                    reason: "after WHERE clause has no expression".to_string(),
                    before: before_expr,
                    after: None,
                }
            }
        },
        None => None,
    };

    if before_expr.as_ref().is_some_and(is_top_level_or) {
        return ConjunctDiff::Opaque {
            reason: "before WHERE clause is a top-level OR, not a conjunctive predicate — a \
                     conjunct-set add/remove framing is unsound for a non-conjunctive rewrite"
                .to_string(),
            before: before_expr,
            after: after_expr,
        };
    }
    if after_expr.as_ref().is_some_and(is_top_level_or) {
        return ConjunctDiff::Opaque {
            reason: "after WHERE clause is a top-level OR, not a conjunctive predicate — a \
                     conjunct-set add/remove framing is unsound for a non-conjunctive rewrite"
                .to_string(),
            before: before_expr,
            after: after_expr,
        };
    }

    let mut before_conjuncts = Vec::new();
    if let Some(e) = &before_expr {
        crate::analysis::expr_util::split_top_level_conjuncts(e, &mut before_conjuncts);
    }
    let mut after_conjuncts = Vec::new();
    if let Some(e) = &after_expr {
        crate::analysis::expr_util::split_top_level_conjuncts(e, &mut after_conjuncts);
    }

    let mut matched_after = vec![false; after_conjuncts.len()];
    let mut unchanged = Vec::new();
    let mut removed = Vec::new();
    for b in &before_conjuncts {
        let found = after_conjuncts
            .iter()
            .enumerate()
            .find(|(i, a)| !matched_after[*i] && same_modulo_trivia(b.syntax(), a.syntax()));
        match found {
            Some((idx, _)) => {
                matched_after[idx] = true;
                unchanged.push(b.clone());
            }
            None => removed.push(b.clone()),
        }
    }
    let added = after_conjuncts
        .into_iter()
        .enumerate()
        .filter(|(i, _)| !matched_after[*i])
        .map(|(_, a)| a)
        .collect();

    ConjunctDiff::Diffed {
        added,
        removed,
        unchanged,
    }
}

/// Whether `expr`'s own outermost operator is a boolean `OR` — the signal
/// for a non-conjunctive top-level predicate (see [`ConjunctDiff::Opaque`]).
fn is_top_level_or(expr: &Expr) -> bool {
    expr.as_binary()
        .is_some_and(|b| b.operator().as_deref() == Some("OR"))
}

// ===== Skeleton (FROM/JOIN tree, GROUP BY, dedup, and the post-processing
// clauses — HAVING/QUALIFY/WINDOW/ORDER BY/LIMIT — that also gate row-set,
// ordering, or row-count semantics) =====

fn skeleton_diff(before: &SelectStmt, after: &SelectStmt) -> SkeletonDiff {
    let before_from = before.from_clause();
    let after_from = after.from_clause();

    let base_equal = match (
        before_from.as_ref().and_then(|f| f.table_refs().next()),
        after_from.as_ref().and_then(|f| f.table_refs().next()),
    ) {
        (Some(b), Some(a)) => same_modulo_trivia(b.syntax(), a.syntax()),
        (None, None) => true,
        _ => false,
    };
    if !base_equal {
        return SkeletonDiff::Changed {
            reason: "the FROM target changed".to_string(),
        };
    }

    let group_by_equal = match (before.group_by_clause(), after.group_by_clause()) {
        (Some(b), Some(a)) => same_modulo_trivia(b.syntax(), a.syntax()),
        (None, None) => true,
        _ => false,
    };
    if !group_by_equal {
        return SkeletonDiff::Changed {
            reason: "GROUP BY changed".to_string(),
        };
    }

    if before.is_distinct() != after.is_distinct() {
        return SkeletonDiff::Changed {
            reason: "DISTINCT toggled".to_string(),
        };
    }

    if let Some(reason) = post_processing_clause_changed(before, after) {
        return SkeletonDiff::Changed { reason };
    }

    let before_joins: Vec<JoinClause> = before_from.iter().flat_map(|f| f.joins()).collect();
    let after_joins: Vec<JoinClause> = after_from.iter().flat_map(|f| f.joins()).collect();

    match diff_joins(&before_joins, &after_joins) {
        JoinDiffResult::Same => SkeletonDiff::Unchanged,
        JoinDiffResult::AddedLeft(added) => SkeletonDiff::AddedLeftJoins(added),
        JoinDiffResult::Changed(reason) => SkeletonDiff::Changed { reason },
    }
}

/// Compare the five post-processing clauses the row-set/skeleton comparison
/// above would otherwise silently ignore: `HAVING`, `QUALIFY`, `WINDOW`,
/// `ORDER BY`, `LIMIT`/`OFFSET`. Each of these gates row-set, ordering, or
/// row-count semantics on its own — research §4's honest-refusals catalogue
/// (G-class) calls out "LIMIT/ORDER BY changes: refuse" specifically — so a
/// difference here must never be folded into an otherwise-`Unchanged`
/// verdict. Returns the reason for the first difference found, or `None` if
/// all five compare equal (modulo trivia) between versions.
///
/// `HAVING`/`QUALIFY` are compared via their single expression (there is no
/// other content in either clause); `WINDOW`/`ORDER BY`/`LIMIT` are compared
/// as whole subtrees. A clause present on one side with no expression at all
/// (a shape this module doesn't expect from a successful parse) never counts
/// as equal to the clause being absent — fail closed rather than guess.
fn post_processing_clause_changed(before: &SelectStmt, after: &SelectStmt) -> Option<String> {
    let having_equal = match (before.having_clause(), after.having_clause()) {
        (None, None) => true,
        (Some(b), Some(a)) => match (b.expression(), a.expression()) {
            (Some(be), Some(ae)) => same_modulo_trivia(be.syntax(), ae.syntax()),
            _ => false,
        },
        _ => false,
    };
    if !having_equal {
        return Some("HAVING changed".to_string());
    }

    let qualify_equal = match (before.qualify_clause(), after.qualify_clause()) {
        (None, None) => true,
        (Some(b), Some(a)) => match (b.expression(), a.expression()) {
            (Some(be), Some(ae)) => same_modulo_trivia(be.syntax(), ae.syntax()),
            _ => false,
        },
        _ => false,
    };
    if !qualify_equal {
        return Some("QUALIFY changed".to_string());
    }

    let window_equal = match (before.window_clause(), after.window_clause()) {
        (None, None) => true,
        (Some(b), Some(a)) => same_modulo_trivia(b.syntax(), a.syntax()),
        _ => false,
    };
    if !window_equal {
        return Some("WINDOW changed".to_string());
    }

    let order_by_equal = match (before.order_by_clause(), after.order_by_clause()) {
        (None, None) => true,
        (Some(b), Some(a)) => same_modulo_trivia(b.syntax(), a.syntax()),
        _ => false,
    };
    if !order_by_equal {
        return Some("ORDER BY changed".to_string());
    }

    let limit_equal = match (before.limit_clause(), after.limit_clause()) {
        (None, None) => true,
        (Some(b), Some(a)) => same_modulo_trivia(b.syntax(), a.syntax()),
        _ => false,
    };
    if !limit_equal {
        return Some("LIMIT/OFFSET changed".to_string());
    }

    None
}

enum JoinDiffResult {
    Same,
    AddedLeft(Vec<JoinClause>),
    Changed(String),
}

/// Whether `before` is an order-preserving subsequence of `after` (each
/// `before` join matched, in order, to a distinct `after` join via
/// [`join_equal`]); if so, the unmatched `after` joins are the added ones.
/// Anything else (a `before` join missing or reordered incompatibly in
/// `after`) is `Changed`; added joins that are not all `LEFT JOIN` are also
/// `Changed` — this module only recognises additive `LEFT JOIN`s.
fn diff_joins(before: &[JoinClause], after: &[JoinClause]) -> JoinDiffResult {
    let mut matched = vec![false; after.len()];
    let mut ai = 0;
    for b in before {
        let mut found = false;
        while ai < after.len() {
            let candidate_idx = ai;
            ai += 1;
            if !matched[candidate_idx] && join_equal(b, &after[candidate_idx]) {
                matched[candidate_idx] = true;
                found = true;
                break;
            }
        }
        if !found {
            return JoinDiffResult::Changed(
                "a join present before is missing, changed, or reordered incompatibly after"
                    .to_string(),
            );
        }
    }

    let extra: Vec<JoinClause> = after
        .iter()
        .zip(matched.iter())
        .filter(|(_, m)| !**m)
        .map(|(j, _)| j.clone())
        .collect();

    if extra.is_empty() {
        JoinDiffResult::Same
    } else if extra.iter().all(|j| j.join_type() == Some(JoinType::Left)) {
        JoinDiffResult::AddedLeft(extra)
    } else {
        JoinDiffResult::Changed("added join(s) are not all LEFT JOIN".to_string())
    }
}

/// Structural equality of two JOIN clauses: same join type, `NATURAL`-ness,
/// joined table, and condition (`ON` expression or `USING` column list).
fn join_equal(a: &JoinClause, b: &JoinClause) -> bool {
    if a.join_type() != b.join_type() {
        return false;
    }
    if a.is_natural() != b.is_natural() {
        return false;
    }
    let tables_equal = match (a.table_ref(), b.table_ref()) {
        (Some(ta), Some(tb)) => same_modulo_trivia(ta.syntax(), tb.syntax()),
        (None, None) => true,
        _ => false,
    };
    if !tables_equal {
        return false;
    }
    match (a.condition(), b.condition()) {
        (None, None) => true,
        (Some(ca), Some(cb)) => {
            if ca.is_on() && cb.is_on() {
                match (ca.on_expression(), cb.on_expression()) {
                    (Some(ea), Some(eb)) => same_modulo_trivia(ea.syntax(), eb.syntax()),
                    (None, None) => true,
                    _ => false,
                }
            } else if ca.is_using() && cb.is_using() {
                ca.using_columns() == cb.using_columns()
            } else {
                false
            }
        }
        _ => false,
    }
}

// ===== Set operations (UNION ALL branches) =====

fn set_op_diff(before: &SelectStmt, after: &SelectStmt) -> SetOpDiff {
    let before_chain = collect_union_all_chain(before);
    let after_chain = collect_union_all_chain(after);

    match (before_chain, after_chain) {
        (Ok(b), Ok(a)) if b.len() == 1 && a.len() == 1 => SetOpDiff::NotApplicable,
        (Ok(b), Ok(a)) => {
            let b_tokens: Vec<_> = b.iter().map(branch_token_seq).collect();
            let a_tokens: Vec<_> = a.iter().map(branch_token_seq).collect();
            let mut matched_after = vec![false; a.len()];
            let mut unchanged = Vec::new();
            // Before-branches that found no exact-text match in `after`,
            // carrying their own *original* (before-chain) position — this
            // is what `EditedBranch::index` reports, since that position is
            // what determines whether the top-level (whole-definition)
            // SELECT-list/WHERE/skeleton diff describes this branch (see
            // `EditedBranch`'s doc comment).
            let mut leftover_before: Vec<(usize, &SelectStmt)> = Vec::new();
            for (bi, bb) in b.iter().enumerate() {
                let found = a_tokens
                    .iter()
                    .enumerate()
                    .find(|(i, at)| !matched_after[*i] && **at == b_tokens[bi]);
                match found {
                    Some((idx, _)) => {
                        matched_after[idx] = true;
                        unchanged.push(bb.clone());
                    }
                    None => leftover_before.push((bi, bb)),
                }
            }
            // After-branches that found no exact-text match, in their
            // original relative order, each carrying its own *after*-chain
            // position (`EditedBranch::after_index`) — `matched_after`
            // preserves position, so the earliest unmatched `after` branch
            // (in particular `after`'s own branch 0, when it changed) is
            // always first here.
            let leftover_after: Vec<(usize, &SelectStmt)> = a
                .iter()
                .enumerate()
                .filter(|(i, _)| !matched_after[*i])
                .collect();

            // Pair the leftovers positionally among themselves — a survivor
            // that changed its own content no longer text-matches, but is
            // still the "same" branch in the sense that matters for
            // classification: research §4 F1's edited-survivor
            // generalization. Any leftover beyond the shorter side's length
            // is a genuine add or remove, not an edit.
            let pair_count = leftover_before.len().min(leftover_after.len());
            let mut edited = Vec::with_capacity(pair_count);
            for i in 0..pair_count {
                let (index, before_branch) = leftover_before[i];
                let (after_index, after_branch) = leftover_after[i];
                edited.push(EditedBranch {
                    index,
                    after_index,
                    select_list: select_list_diff(
                        before_branch.select_list().as_ref(),
                        after_branch.select_list().as_ref(),
                    ),
                    where_clause: where_diff(before_branch, after_branch),
                    skeleton: skeleton_diff(before_branch, after_branch),
                });
            }
            let removed: Vec<SelectStmt> = leftover_before[pair_count..]
                .iter()
                .map(|(_, s)| (*s).clone())
                .collect();
            let added: Vec<SelectStmt> = leftover_after[pair_count..]
                .iter()
                .map(|(_, s)| (*s).clone())
                .collect();

            SetOpDiff::Branches {
                added,
                removed,
                unchanged,
                edited,
            }
        }
        (Err(reason), _) | (_, Err(reason)) => SetOpDiff::Opaque { reason },
    }
}

/// Walk `stmt`'s set-operation chain, requiring every link to be a plain
/// `UNION ALL` (not `UNION`/`INTERSECT`/`EXCEPT`, not `BY NAME`). Returns
/// `Ok([stmt])` when there is no set operation at all. Fails closed with a
/// reason for any other set-operation shape — this module does not attempt
/// to diff dedup-semantics (`UNION`), different-algebra operators
/// (`INTERSECT`/`EXCEPT`), or name-unified (`BY NAME`) operands.
fn collect_union_all_chain(stmt: &SelectStmt) -> Result<Vec<SelectStmt>, String> {
    let mut chain = vec![stmt.clone()];
    let mut current = stmt.clone();
    while current.has_set_operation() {
        if current.is_set_operation_by_name() {
            return Err("a `BY NAME` set operation is not diffed by the pure module".to_string());
        }
        if !(current.has_union() && current.is_union_all()) {
            return Err(
                "a set operation other than UNION ALL is not diffed by the pure module".to_string(),
            );
        }
        let next = current
            .set_operation_select()
            .ok_or_else(|| "a UNION ALL has no right-hand SELECT".to_string())?;
        chain.push(next.clone());
        current = next;
    }
    Ok(chain)
}

/// Trivia-insensitive token sequence for *just* `stmt`'s own branch content
/// (its `SELECT` list / `FROM` / `WHERE` / `GROUP BY` / … clauses), used to
/// compare set-operation branches for the multiset diff above.
///
/// This deliberately does **not** reuse [`same_modulo_trivia`] on
/// `stmt.syntax()` directly: the grammar parses `A UNION ALL B` with `B`
/// (and every branch after it) nested *inside* `A`'s own `SELECT_STMT` node
/// (`parse_set_op_tail` runs before `A`'s node closes), so `A.syntax()`'s
/// descendant tokens include the entire remainder of the chain, not just
/// branch `A`. Walking `stmt`'s direct children and stopping at the first
/// `UNION`/`INTERSECT`/`EXCEPT` token isolates this branch's own tokens from
/// the next branch it introduces.
fn branch_token_seq(stmt: &SelectStmt) -> Vec<(smelt_parser::SyntaxKind, String)> {
    use smelt_parser::SyntaxKind::{EXCEPT_KW, INTERSECT_KW, UNION_KW};

    let mut out = Vec::new();
    for child in stmt.syntax().children_with_tokens() {
        if let Some(tok) = child.as_token() {
            if matches!(tok.kind(), UNION_KW | INTERSECT_KW | EXCEPT_KW) {
                // The set-op tail (and the next branch it introduces) starts
                // here — everything from here on belongs to later branches.
                break;
            }
            if !tok.kind().is_trivia() {
                out.push((tok.kind(), tok.text().to_string()));
            }
            continue;
        }
        if let Some(node) = child.as_node() {
            out.extend(
                node.descendants_with_tokens()
                    .filter_map(|e| e.into_token())
                    .filter(|t| !t.kind().is_trivia())
                    .map(|t| (t.kind(), t.text().to_string())),
            );
        }
    }
    out
}
