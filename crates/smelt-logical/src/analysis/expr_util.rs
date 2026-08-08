//! Shared expression-tree primitives used across `analysis/`,
//! `maintenance/`, and `backbuild/`: collecting the column references
//! embedded in an already-parsed expression, and splitting a `WHERE`/`ON`
//! predicate into its top-level `AND`-joined conjuncts.
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

/// The pre-unification, un-gated shape of [`collect_column_refs`] —
/// preserved verbatim for the one caller
/// ([`crate::maintenance::grouping`]'s per-column provenance derivation)
/// where fixing the bug changes an admission verdict
/// (`maintenance_conformance::keyed_enriched_pool_upholds_equivalence_with_zero_write_redelivery`
/// picks a different maintenance technique once the fix is applied, because
/// a function-call-wrapped column reference starts contributing to
/// provenance that today it silently doesn't). Phase 2's contract is
/// behaviour preservation only — no admission verdict, plan cell, or
/// emitted statement may change — so this caller keeps the bug for now;
/// seeing it fixed is tracked in `docs/TODO.md` as a follow-up
/// admission-widening change (would need the same oracle verification
/// discipline as Phases 3/5's named accepting-direction changes).
pub(crate) fn collect_column_refs_ungated(expr: &Expr) -> Vec<ColumnRef> {
    let mut out = Vec::new();
    collect_column_refs_ungated_rec(expr.syntax(), &mut out);
    out
}

fn collect_column_refs_ungated_rec(node: &SyntaxNode, out: &mut Vec<ColumnRef>) {
    if let Some(e) = Expr::cast(node.clone()) {
        if let Some(cref) = ColumnRef::from_expr(&e) {
            out.push(cref);
            return;
        }
    }
    for child in node.children() {
        collect_column_refs_ungated_rec(&child, out);
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
    /// `OVER` clauses, `CAST`, `BETWEEN` — table-driven over both
    /// `collect_column_refs` (today's crate-wide reconciled behaviour: the
    /// `EXPRESSION`-gated shape, adopted from `analysis::fingerprint`'s
    /// pre-unification copy) and `collect_column_refs_ungated` (the
    /// `skeleton_closure`/`grouping` pre-unification shape, preserved only
    /// for `maintenance::grouping` — see `docs/TODO.md`).
    ///
    /// The two disagree on any expression whose outermost or nested form is
    /// a bare function call: `collect_column_refs_ungated` misreads the
    /// call's own name as a bare column reference *and* — because the
    /// match short-circuits the recursion — drops every real column
    /// reference among that call's arguments. `collect_column_refs` (gated)
    /// does neither.
    #[test]
    fn column_ref_collection_characterization() {
        // Qualified ref — no function call anywhere; both agree.
        assert_eq!(
            as_tuples(&collect_column_refs(&expr("a.b"))),
            refs(&[(Some("a"), "b")])
        );
        assert_eq!(
            as_tuples(&collect_column_refs_ungated(&expr("a.b"))),
            refs(&[(Some("a"), "b")])
        );

        // Bare ref — both agree.
        assert_eq!(
            as_tuples(&collect_column_refs(&expr("a"))),
            refs(&[(None, "a")])
        );
        assert_eq!(
            as_tuples(&collect_column_refs_ungated(&expr("a"))),
            refs(&[(None, "a")])
        );

        // CASE / aliases — no function call; both agree.
        let case_sql = "CASE WHEN a.x > 1 THEN a.y ELSE b.z END";
        let case_expected = refs(&[(Some("a"), "x"), (Some("a"), "y"), (Some("b"), "z")]);
        assert_eq!(
            as_tuples(&collect_column_refs(&expr(case_sql))),
            case_expected
        );
        assert_eq!(
            as_tuples(&collect_column_refs_ungated(&expr(case_sql))),
            case_expected
        );

        // Function args — DISAGREEMENT: gated correctly finds both
        // arguments and no spurious `foo` entry; ungated finds only a
        // spurious bare `foo` column and drops both real arguments.
        assert_eq!(
            as_tuples(&collect_column_refs(&expr("foo(a.x, b.y)"))),
            refs(&[(Some("a"), "x"), (Some("b"), "y")])
        );
        assert_eq!(
            as_tuples(&collect_column_refs_ungated(&expr("foo(a.x, b.y)"))),
            refs(&[(None, "foo")])
        );

        // Window OVER clause — DISAGREEMENT: same shape as function args;
        // the ungated copy also loses the aggregate's own argument (`a.x`)
        // while still finding the PARTITION BY/ORDER BY refs (siblings of
        // the FUNCTION_CALL node, not inside it).
        let over_sql = "SUM(a.x) OVER (PARTITION BY a.y ORDER BY a.z)";
        assert_eq!(
            as_tuples(&collect_column_refs(&expr(over_sql))),
            refs(&[(Some("a"), "x"), (Some("a"), "y"), (Some("a"), "z")])
        );
        assert_eq!(
            as_tuples(&collect_column_refs_ungated(&expr(over_sql))),
            refs(&[(None, "SUM"), (Some("a"), "y"), (Some("a"), "z")])
        );

        // CAST — no function call; both agree.
        assert_eq!(
            as_tuples(&collect_column_refs(&expr("CAST(a.x AS INTEGER)"))),
            refs(&[(Some("a"), "x")])
        );
        assert_eq!(
            as_tuples(&collect_column_refs_ungated(&expr("CAST(a.x AS INTEGER)"))),
            refs(&[(Some("a"), "x")])
        );

        // BETWEEN — no function call; both agree.
        let between_expected = refs(&[(Some("a"), "x"), (Some("b"), "y"), (Some("b"), "z")]);
        assert_eq!(
            as_tuples(&collect_column_refs(&expr("a.x BETWEEN b.y AND b.z"))),
            between_expected
        );
        assert_eq!(
            as_tuples(&collect_column_refs_ungated(&expr(
                "a.x BETWEEN b.y AND b.z"
            ))),
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
