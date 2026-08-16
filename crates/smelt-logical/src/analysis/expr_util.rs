//! Shared expression-tree primitives used across `analysis/`,
//! `maintenance/`, and `backbuild/`: collecting the column references
//! embedded in an already-parsed expression, splitting a `WHERE`/`ON`
//! predicate into its top-level `AND`-joined conjuncts, and comparing two
//! syntax subtrees for trivia-insensitive structural equality.
//!
//! Before this module existed, both primitives were copied independently at
//! each call site (`analysis::fingerprint`, `analysis::skeleton_closure`,
//! `maintenance::grouping`, `backbuild::classify` for column-ref collection;
//! `analysis::walk`, `backbuild::diff`, `backbuild::classify` for conjunct
//! splitting) and two of the four column-ref copies (`skeleton_closure`,
//! `grouping`) had silently diverged from the other two: they omitted the
//! `EXPRESSION`-kind guard below, so a bare function call anywhere in the
//! expression (e.g. `SUM(a.x) OVER (...)`, `foo(a.x, b.y)`) was
//! misidentified as itself a column reference (its call-name read as a bare
//! column) *and*, because the match short-circuits the recursion, every
//! genuine column reference among that call's own arguments was silently
//! dropped. See `crates/smelt-logical/tests/expr_util.rs` for the
//! characterization that caught this and the reconciliation: this module's
//! `collect_column_refs` is the guarded (correct) shape, adopted crate-wide.

use std::collections::{BTreeSet, HashSet};

use smelt_parser::syntax_kind::SyntaxNode;
use smelt_parser::{ColumnRef, Expr, SyntaxKind};

/// Recursively collect every simple (possibly qualified) column reference
/// inside `expr` — a leaf classifier over one already-parsed expression's
/// own syntax tree, never a raw-text scan.
///
/// Only a genuine `EXPRESSION` wrapper node is a candidate bare/qualified
/// column reference. `Expr::cast` also accepts a `FUNCTION_CALL` node (and
/// several other non-`EXPRESSION` kinds) directly, so its callable body can
/// be inspected as an expression in its own right elsewhere — but
/// `ColumnRef::from_expr` would then misread the function's own
/// single-`IDENT` name token as a bare column reference if this recursion
/// tried to cast a `FUNCTION_CALL` node itself. Restricting the cast
/// attempt to `EXPRESSION` nodes avoids that false match while still
/// finding every legitimate reference — always wrapped in an `EXPRESSION`
/// node at the point it appears as a select-item or argument expression —
/// and, critically, still recurses into a function call's own arguments
/// (which the false match would otherwise short-circuit past).
pub(crate) fn collect_column_refs(expr: &Expr) -> Vec<ColumnRef> {
    let mut out = Vec::new();
    collect_column_refs_rec(expr.syntax(), &mut out);
    out
}

fn collect_column_refs_rec(node: &SyntaxNode, out: &mut Vec<ColumnRef>) {
    if node.kind() == SyntaxKind::EXPRESSION {
        if let Some(e) = Expr::cast(node.clone()) {
            if let Some(cref) = ColumnRef::from_expr(&e) {
                out.push(cref);
                return;
            }
        }
    }
    for child in node.children() {
        collect_column_refs_rec(&child, out);
    }
}

/// Every bare column *name* `expr` references anywhere in its subtree,
/// discarding qualifiers — a thin wrapper over [`collect_column_refs`] for
/// callers that only need the name set (e.g. a disjointness probe over two
/// conjuncts' referenced columns).
pub(crate) fn collect_column_names(expr: &Expr) -> HashSet<String> {
    collect_column_refs(expr)
        .into_iter()
        .map(|cref| cref.name().to_string())
        .collect()
}

/// Every qualifier `expr` references anywhere in its subtree — a thin
/// wrapper over [`collect_column_refs`] for callers that only need the
/// qualifier set (e.g. building a reference-dependency graph over FROM-tree
/// aliases). Unqualified (bare) references contribute nothing.
pub(crate) fn collect_referenced_qualifiers(expr: &Expr) -> BTreeSet<String> {
    collect_column_refs(expr)
        .into_iter()
        .filter_map(|cref| cref.qualifier().map(|q| q.to_string()))
        .collect()
}

/// Trivia-insensitive structural equality: two syntax subtrees are equal
/// when their non-trivia token kind+text sequences match. Whitespace and
/// comments are skipped; token text is compared exactly (case-preserving —
/// a case *change* is not a no-op).
///
/// The single token-stream equality primitive for definition diffs
/// (`docs/plans/20260808-substrate-unification.md` Phase 4): both
/// `backbuild::diff`'s clause-by-clause `DefinitionDiff` comparators and
/// `analysis::model_diff`'s `additive_only_diff` column-expression
/// comparison consume this rather than each carrying their own notion of
/// "unchanged" (previously `backbuild::diff` had a private token-stream
/// copy and `model_diff` used raw `.text().trim()`, which disagreed on a
/// pure whitespace/comment reformat).
pub(crate) fn same_modulo_trivia(a: &SyntaxNode, b: &SyntaxNode) -> bool {
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

/// Recursively split `expr` into its top-level `AND`-joined conjuncts.
/// `AND` is left-associative in the grammar (`a AND b AND c` parses as
/// `(a AND b) AND c`), so this recurses into both operands whenever the
/// operator is `AND`; anything else (including a nested `OR` or a
/// `BETWEEN ... AND ...`, whose `AND` lives inside its own `BETWEEN_EXPR`
/// node rather than a `BINARY_EXPR`) is a single atomic conjunct, pushed as
/// a whole unit.
pub(crate) fn split_top_level_conjuncts(expr: &Expr, out: &mut Vec<Expr>) {
    if let Some(bin) = expr.as_binary() {
        if bin.operator().as_deref() == Some("AND") {
            if let (Some(l), Some(r)) = (bin.left(), bin.right()) {
                split_top_level_conjuncts(&l, out);
                split_top_level_conjuncts(&r, out);
                return;
            }
        }
    }
    out.push(expr.clone());
}

/// Characterization tests documenting the pre-unification per-copy
/// behaviour and the reconciliation decision (`docs/plans/
/// 20260808-substrate-unification.md` Phase 2). These are unit tests
/// (rather than the `tests/expr_util.rs` integration test the plan names)
/// because `collect_column_refs`/`split_top_level_conjuncts` are
/// deliberately `pub(crate)`, not part of the crate's external API surface
/// — only same-crate code can call them directly.
/// `crates/smelt-logical/tests/expr_util.rs` covers the same battery
/// black-box, through the public call sites that consume these helpers.
#[cfg(test)]
mod tests {
    use super::*;

    fn expr(sql_expr: &str) -> Expr {
        let sql = format!("SELECT {sql_expr} AS v FROM t");
        let parse = smelt_parser::parse(&sql);
        let file = smelt_parser::File::cast(parse.syntax()).expect("file");
        let select = file.select_stmt().expect("select");
        select
            .select_list()
            .expect("select list")
            .items()
            .next()
            .expect("item")
            .expression()
            .expect("expression")
    }

    fn as_tuples(v: &[ColumnRef]) -> Vec<(Option<String>, String)> {
        v.iter()
            .map(|c| (c.qualifier().map(|q| q.to_string()), c.name().to_string()))
            .collect()
    }

    fn refs(v: &[(Option<&str>, &str)]) -> Vec<(Option<String>, String)> {
        v.iter()
            .map(|(q, n)| (q.map(|s| s.to_string()), n.to_string()))
            .collect()
    }

    /// Battery: qualified refs, bare refs, `CASE`, function args, window
    /// `OVER` clauses, `CAST`, `BETWEEN` — the `EXPRESSION`-gated shape
    /// adopted crate-wide (`analysis::fingerprint`'s pre-unification copy):
    /// only a genuine `EXPRESSION` wrapper node is a candidate column
    /// reference, so a bare function call's own name is never misread as a
    /// column and every real reference among its arguments is still found.
    #[test]
    fn column_ref_collection_characterization() {
        // Qualified ref.
        assert_eq!(
            as_tuples(&collect_column_refs(&expr("a.b"))),
            refs(&[(Some("a"), "b")])
        );

        // Bare ref.
        assert_eq!(
            as_tuples(&collect_column_refs(&expr("a"))),
            refs(&[(None, "a")])
        );

        // CASE / aliases.
        let case_sql = "CASE WHEN a.x > 1 THEN a.y ELSE b.z END";
        let case_expected = refs(&[(Some("a"), "x"), (Some("a"), "y"), (Some("b"), "z")]);
        assert_eq!(
            as_tuples(&collect_column_refs(&expr(case_sql))),
            case_expected
        );

        // Function args — the gated shape correctly finds both arguments
        // and no spurious `foo` entry.
        assert_eq!(
            as_tuples(&collect_column_refs(&expr("foo(a.x, b.y)"))),
            refs(&[(Some("a"), "x"), (Some("b"), "y")])
        );

        // Window OVER clause — same shape as function args.
        let over_sql = "SUM(a.x) OVER (PARTITION BY a.y ORDER BY a.z)";
        assert_eq!(
            as_tuples(&collect_column_refs(&expr(over_sql))),
            refs(&[(Some("a"), "x"), (Some("a"), "y"), (Some("a"), "z")])
        );

        // CAST.
        assert_eq!(
            as_tuples(&collect_column_refs(&expr("CAST(a.x AS INTEGER)"))),
            refs(&[(Some("a"), "x")])
        );

        // BETWEEN.
        let between_expected = refs(&[(Some("a"), "x"), (Some("b"), "y"), (Some("b"), "z")]);
        assert_eq!(
            as_tuples(&collect_column_refs(&expr("a.x BETWEEN b.y AND b.z"))),
            between_expected
        );
    }

    /// Battery: nested parens, `OR` guards, `BETWEEN` — table-driven over
    /// the reconciled `split_top_level_conjuncts` (unifying `backbuild::
    /// diff`'s `split_conjuncts` and `backbuild::classify`'s
    /// `split_top_level_and`, which were byte-identical in logic — no
    /// disagreement to reconcile there).
    #[test]
    fn conjunct_split_characterization() {
        fn split(sql_expr: &str) -> Vec<String> {
            let mut out = Vec::new();
            split_top_level_conjuncts(&expr(sql_expr), &mut out);
            out.iter().map(|e| e.text().trim().to_string()).collect()
        }

        // Plain two-way AND.
        assert_eq!(split("a = 1 AND b = 2"), vec!["a = 1", "b = 2"]);

        // Left-associative three-way AND flattens fully.
        assert_eq!(
            split("a = 1 AND b = 2 AND c = 3"),
            vec!["a = 1", "b = 2", "c = 3"]
        );

        // Nested parens around a conjunct don't themselves force a split —
        // `(a = 1 AND b = 2)` is one parenthesized AND expression; splitting
        // recurses through the parens into the AND it wraps.
        assert_eq!(
            split("(a = 1 AND b = 2) AND c = 3"),
            vec!["a = 1", "b = 2", "c = 3"]
        );

        // OR guard — an OR is never split; it's one atomic conjunct even
        // when AND-joined with something else.
        assert_eq!(
            split("a = 1 AND (b = 2 OR c = 3)"),
            vec!["a = 1", "b = 2 OR c = 3"]
        );

        // BETWEEN's own AND lives inside BETWEEN_EXPR, not a BINARY_EXPR —
        // never split, stays one atomic conjunct.
        assert_eq!(
            split("a BETWEEN 1 AND 10 AND b = 2"),
            vec!["a BETWEEN 1 AND 10", "b = 2"]
        );
    }
}
