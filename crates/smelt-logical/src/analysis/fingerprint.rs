//! Fingerprint-projection proof (P4, `model_properties.md` §"Fingerprint
//! projection"): for a consuming model reading an external source, which of
//! the source's columns feed the model's output — the column set a
//! row-content fingerprint sidecar (`docs/specs/sources.md` §"The
//! fingerprint sidecar") digests instead of the source row's full content.
//!
//! **Walk-composed, not a whole-text scan.** The derivation reuses the
//! shared composition walk's (`analysis::walk`) already-resolved per-column
//! lineage: `walk::QueryTree`'s root `SELECT` scope carries a
//! [`walk::ColumnLineage`] for every projected output column, and a simple
//! rename chain (a bare, possibly-qualified column reference, chased
//! through arbitrarily nested CTEs and derived tables by the walk's own
//! `resolve_leaf`) already resolves to a concrete base-relation column —
//! this is exactly how the CTE-composed case is handled, with no extra code
//! here. A column whose expression is not a simple rename (a computed
//! expression, a function call) is inspected by a **leaf classifier** over
//! that one already-bounded expression's own AST (never a raw-text scan):
//! every embedded column reference is resolved against the scope's own
//! FROM-alias map, and every function call is checked against the shared
//! function registry (`smelt_types::SqlFunction`) for opaqueness.
//!
//! **Fail-closed** (`model_properties.md` §"Fingerprint projection"): a bare
//! or qualified `SELECT *`/`<alias>.*` that could read the source in
//! question, an opaque (registry-unrecognised) function call applied over a
//! column of the source, an embedded column reference this leaf classifier
//! cannot resolve (an ambiguous unqualified reference, a reference into a
//! CTE/derived-table alias this classifier does not chase further), a
//! top-level set-operation root, or any construct the walk itself could not
//! normalize all yield [`Projection::FullRow`] naming the reason, never a
//! guessed subset.
//!
//! **v1 scope restriction**: only the model's own outermost `SELECT` scope
//! is classified narrowly — matching the same restriction
//! `maintenance::grouping`'s per-column mutation-sensitivity classifier
//! documents for its non-walk-composed sibling. Unlike that classifier,
//! CTE/derived-table composition through a *simple rename* is not a
//! restriction here (the walk resolves it); only an embedded reference
//! *inside a computed expression* that reaches into a CTE/derived-table
//! alias remains unresolved and fails closed.

use std::collections::BTreeSet;

use smelt_parser::syntax_kind::SyntaxNode;
use smelt_parser::{ColumnRef, Expr};

use super::walk::{LeafInput, NodeCx, OpNode, QueryNode, QueryTree, RelationSource, Transfer};

/// The P4 verdict: either the concrete set of the source's columns that
/// feed the model's output, or a fail-closed full-row digest naming why a
/// concrete projection could not be established.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Projection {
    /// The exact set of `source_ref` columns whose value feeds the model's
    /// output — the sidecar digests only these.
    Columns(BTreeSet<String>),
    /// Fail-closed: the sidecar must digest the source's full row.
    FullRow { reason: String },
}

impl Projection {
    pub fn is_full_row(&self) -> bool {
        matches!(self, Projection::FullRow { .. })
    }
}

/// Canonical identity string for a P4 projection verdict — the namespace
/// key a fingerprint sidecar partitions by
/// (`docs/specs/sources.md` §"The fingerprint sidecar" — "Naming and
/// namespace": "A row is namespaced by `(source address, projection
/// identity, source key)` — projection identity, not consumer identity").
/// Two consumers whose derived [`Projection::Columns`] sets are
/// byte-identical incidentally land on the same identity (a storage
/// optimisation, never a correctness dependency, per that same section);
/// a [`Projection::FullRow`] fail-closed verdict always gets its own
/// distinct identity, disjoint from every `Columns` identity even one that
/// happens to enumerate the exact same column names — "it is never
/// silently widened onto, or narrowed from, another consumer's own
/// projection". The `reason` a `FullRow` verdict carries is deliberately
/// NOT part of the identity: every fail-closed cause shares one full-row
/// sidecar partition, since they all mean the same thing operationally
/// (digest the whole row).
pub fn projection_identity(projection: &Projection) -> String {
    match projection {
        Projection::Columns(cols) => {
            format!(
                "cols:{}",
                cols.iter().cloned().collect::<Vec<_>>().join(",")
            )
        }
        Projection::FullRow { .. } => "full_row".to_string(),
    }
}

/// Derive the fingerprint projection (P4) of `model_sql` over `source_ref`
/// — a `smelt.<path>` source name, `sources.`-prefix optional, matching
/// [`super::source_bounds::resolve_table_ref_source_name`]'s output
/// convention (the same convention `skeleton_closure::skeleton_source_closure`
/// uses for its own `enrichment_source` parameter).
pub fn fingerprint_projection(model_sql: &str, source_ref: &str) -> Projection {
    let Some(tree) = QueryTree::from_sql(model_sql) else {
        return Projection::FullRow {
            reason: "model SQL has no top-level SELECT to classify".to_string(),
        };
    };
    if tree.root.has_unsupported() {
        return Projection::FullRow {
            reason: "model SQL contains a construct the composition walk cannot normalize"
                .to_string(),
        };
    }
    match &tree.root {
        QueryNode::Select(_) => {
            let transfer = FingerprintTransfer { source_ref };
            super::walk::walk(&tree, &transfer)
        }
        QueryNode::SetOp(_) => Projection::FullRow {
            reason: "model SQL's outermost query is a set operation — fingerprint projection is \
                     not yet classified across set-operation arms"
                .to_string(),
        },
        // Ruled out above by `has_unsupported()`, but matched explicitly
        // rather than a wildcard so a future `QueryNode` variant is not
        // silently swallowed here.
        QueryNode::Unsupported { reason } => Projection::FullRow {
            reason: format!("unsupported construct: {reason}"),
        },
    }
}

struct FingerprintTransfer<'a> {
    source_ref: &'a str,
}

impl Transfer for FingerprintTransfer<'_> {
    type Verdict = Projection;

    fn leaf(&self, _leaf: &LeafInput<'_>, _cx: &NodeCx) -> Projection {
        // A base relation contributes no projection of its own — the
        // consuming scope's own projected columns are what is classified.
        Projection::Columns(BTreeSet::new())
    }

    fn operator(&self, op: &OpNode<'_>, _children: &[Projection], cx: &NodeCx) -> Projection {
        match op {
            OpNode::Unsupported { reason } => Projection::FullRow {
                reason: format!("unsupported construct: {reason}"),
            },
            OpNode::SetOp(_) => Projection::FullRow {
                reason: "set-operation scope — fingerprint projection is not yet classified \
                         across arms"
                    .to_string(),
            },
            OpNode::Select(sn) => select_projection(&sn.select, cx, self.source_ref),
        }
    }
}

/// Whether `relation`'s [`super::walk`] source name resolves to `source_ref`
/// — the same bare/`sources.`-prefixed comparison
/// `skeleton_closure::find_enrichment_join` uses.
fn relation_matches_source(relation: &str, source_ref: &str) -> bool {
    let bare = relation.strip_prefix("sources.").unwrap_or(relation);
    bare.eq_ignore_ascii_case(source_ref) || relation.eq_ignore_ascii_case(source_ref)
}

fn relation_source_matches(source: &RelationSource, source_ref: &str) -> bool {
    match source {
        RelationSource::Table(name) => relation_matches_source(name, source_ref),
        // A CTE/derived-table alias may transitively read `source_ref`;
        // this leaf classifier does not chase into it for a wildcard or an
        // embedded (non-simple-rename) reference — fail closed rather than
        // guess (see module doc).
        RelationSource::Cte(_) | RelationSource::DerivedTable(_) => false,
    }
}

/// How one embedded column reference (inside a computed expression) relates
/// to `source_ref`, resolved against this scope's own FROM-alias map only
/// (no further CTE/derived-table chase — see module doc).
enum RefClass {
    /// Resolves to `source_ref`'s own column `String`.
    Source(String),
    /// Resolves to a different relation entirely.
    Other,
    /// Could not be resolved cleanly (ambiguous unqualified reference, or a
    /// qualifier reaching into a CTE/derived-table alias).
    Unresolved,
}

fn classify_ref(cref: &ColumnRef, cx: &NodeCx, source_ref: &str) -> RefClass {
    let key = match cref.qualifier() {
        Some(q) => q.to_ascii_lowercase(),
        None => {
            if cx.aliases.len() == 1 {
                match cx.aliases.keys().next() {
                    Some(k) => k.clone(),
                    None => return RefClass::Unresolved,
                }
            } else {
                return RefClass::Unresolved;
            }
        }
    };
    match cx.aliases.get(&key) {
        Some(source) => {
            if relation_source_matches(source, source_ref) {
                RefClass::Source(cref.name().to_string())
            } else if matches!(source, RelationSource::Table(_)) {
                RefClass::Other
            } else {
                RefClass::Unresolved
            }
        }
        None => RefClass::Unresolved,
    }
}

/// Recursively collect every simple (possibly qualified) column reference
/// inside `expr` — mirrors `skeleton_closure`'s private helper of the same
/// shape (kept local; that one is not `pub`).
fn collect_column_refs(expr: &Expr) -> Vec<ColumnRef> {
    let mut out = Vec::new();
    collect_column_refs_rec(expr.syntax(), &mut out);
    out
}

fn collect_column_refs_rec(node: &SyntaxNode, out: &mut Vec<ColumnRef>) {
    // Only a genuine `EXPRESSION` wrapper node is a candidate bare/qualified
    // column reference — `Expr::cast` also accepts a `FUNCTION_CALL` node
    // directly (so its callable body can be inspected as an expression),
    // and `ColumnRef::from_expr` would then misread the function's own
    // single-IDENT name token as a bare column reference. Restricting the
    // cast attempt to `EXPRESSION` nodes avoids that false match while
    // still finding every legitimate reference, which is always wrapped in
    // one at the point it appears as a select-item or argument expression.
    if node.kind() == smelt_parser::SyntaxKind::EXPRESSION {
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

/// The result of scanning one non-simple-rename projected expression for
/// provenance touching `source_ref`.
enum ExprTouch {
    /// Nothing in the expression resolves to `source_ref`.
    None,
    /// Exactly the `source_ref` columns this expression's value depends on.
    Columns(BTreeSet<String>),
    /// Fail-closed: an opaque function call or an unresolved reference.
    Refuse(String),
}

/// Leaf classifier over one already-bounded projected expression's own AST
/// (never a raw-text scan): every function call it contains is checked
/// against the shared function registry for opaqueness, and every embedded
/// column reference is classified against `source_ref`.
fn scan_expr_for_source(expr: &Expr, cx: &NodeCx, source_ref: &str) -> ExprTouch {
    // An opaque (registry-unrecognised) function call applied over a column
    // of `source_ref` is fail-closed regardless of what else the expression
    // contains — smelt has no basis to claim a narrower dependency than the
    // function's whole argument.
    for node in expr.syntax().descendants() {
        if node.kind() != smelt_parser::SyntaxKind::FUNCTION_CALL {
            continue;
        }
        let Some(func) = smelt_parser::FunctionCall::cast(node) else {
            continue;
        };
        let Some(name) = func.name() else { continue };
        let recognized = smelt_types::SqlFunction::from_name(&name.to_ascii_uppercase()).is_some();
        if recognized {
            continue;
        }
        for arg in func.arguments() {
            for cref in collect_column_refs(&arg) {
                match classify_ref(&cref, cx, source_ref) {
                    RefClass::Source(_) => {
                        return ExprTouch::Refuse(format!(
                            "opaque function call '{name}' applied over a column of '{source_ref}'"
                        ));
                    }
                    RefClass::Unresolved => {
                        return ExprTouch::Refuse(format!(
                            "unresolvable provenance inside opaque function call '{name}'"
                        ));
                    }
                    RefClass::Other => {}
                }
            }
        }
    }

    let mut cols = BTreeSet::new();
    let mut touched = false;
    for cref in collect_column_refs(expr) {
        match classify_ref(&cref, cx, source_ref) {
            RefClass::Source(col) => {
                cols.insert(col);
                touched = true;
            }
            RefClass::Other => {}
            RefClass::Unresolved => {
                return ExprTouch::Refuse(format!(
                    "unresolvable column reference '{}' — provenance for '{source_ref}' cannot \
                     be established",
                    cref.name()
                ));
            }
        }
    }
    if touched {
        ExprTouch::Columns(cols)
    } else {
        ExprTouch::None
    }
}

/// The fingerprint projection of one `SELECT` scope's own projected columns
/// against `source_ref`. Called by the walk for every scope (CTE bodies
/// included, per the `Transfer` contract) — only the root scope's returned
/// verdict is read by [`fingerprint_projection`], but every scope is
/// classified identically.
fn select_projection(
    select: &smelt_parser::SelectStmt,
    cx: &NodeCx,
    source_ref: &str,
) -> Projection {
    let Some(select_list) = select.select_list() else {
        return Projection::Columns(BTreeSet::new());
    };
    let mut cols = BTreeSet::new();
    let mut idx = 0usize;
    for item in select_list.items() {
        if item.is_wildcard() {
            match item.qualified_wildcard_target() {
                Some(qualifier) => {
                    let key = qualifier.to_ascii_lowercase();
                    match cx.aliases.get(&key) {
                        Some(source) if relation_source_matches(source, source_ref) => {
                            return Projection::FullRow {
                                reason: format!(
                                    "`{qualifier}.*` reads the full row of '{source_ref}'"
                                ),
                            };
                        }
                        Some(RelationSource::Table(_)) => {
                            // A different base relation entirely — doesn't
                            // touch source_ref.
                        }
                        _ => {
                            return Projection::FullRow {
                                reason: format!(
                                    "`{qualifier}.*` targets an alias this classifier cannot \
                                     resolve against '{source_ref}'"
                                ),
                            };
                        }
                    }
                }
                None => {
                    return Projection::FullRow {
                        reason: "`SELECT *` reads the full row of every input; provenance is \
                                 not isolated per source"
                            .to_string(),
                    };
                }
            }
            idx += 1;
            continue;
        }
        let Some(expr) = item.expression() else {
            continue;
        };
        let leaf = cx.columns.get(idx).and_then(|c| c.leaf.clone());
        idx += 1;

        if let Some(leaf) = leaf {
            if relation_matches_source(&leaf.relation, source_ref) {
                cols.insert(leaf.column);
            }
            continue;
        }

        match scan_expr_for_source(&expr, cx, source_ref) {
            ExprTouch::None => {}
            ExprTouch::Columns(found) => cols.extend(found),
            ExprTouch::Refuse(reason) => return Projection::FullRow { reason },
        }
    }
    Projection::Columns(cols)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_column_reads_project_exactly_those_columns() {
        let sql = "SELECT f.id, d.name, d.tier FROM smelt.sources.fact f \
                    JOIN smelt.sources.dim d ON f.dim_id = d.id";
        let verdict = fingerprint_projection(sql, "dim");
        let expected: BTreeSet<String> = ["name".to_string(), "tier".to_string()]
            .into_iter()
            .collect();
        assert_eq!(verdict, Projection::Columns(expected), "{verdict:?}");
    }

    #[test]
    fn bare_select_star_fails_closed_to_full_row() {
        let sql = "SELECT * FROM smelt.sources.dim";
        let verdict = fingerprint_projection(sql, "dim");
        assert!(verdict.is_full_row(), "{verdict:?}");
    }

    #[test]
    fn qualified_wildcard_on_source_fails_closed_to_full_row() {
        let sql = "SELECT f.id, d.* FROM smelt.sources.fact f \
                    JOIN smelt.sources.dim d ON f.dim_id = d.id";
        let verdict = fingerprint_projection(sql, "dim");
        assert!(verdict.is_full_row(), "{verdict:?}");
    }

    #[test]
    fn qualified_wildcard_on_other_relation_does_not_force_full_row() {
        let sql = "SELECT f.*, d.name FROM smelt.sources.fact f \
                    JOIN smelt.sources.dim d ON f.dim_id = d.id";
        let verdict = fingerprint_projection(sql, "dim");
        let expected: BTreeSet<String> = ["name".to_string()].into_iter().collect();
        assert_eq!(verdict, Projection::Columns(expected), "{verdict:?}");
    }

    #[test]
    fn opaque_function_over_source_fails_closed_to_full_row() {
        let sql = "SELECT f.id, custom_udf(d.name) AS x FROM smelt.sources.fact f \
                    JOIN smelt.sources.dim d ON f.dim_id = d.id";
        let verdict = fingerprint_projection(sql, "dim");
        assert!(verdict.is_full_row(), "{verdict:?}");
        if let Projection::FullRow { reason } = verdict {
            assert!(
                reason.contains("opaque function"),
                "expected an opaque-function reason, got: {reason}"
            );
        }
    }

    #[test]
    fn recognized_function_over_source_column_still_projects_narrowly() {
        let sql = "SELECT f.id, UPPER(d.name) AS x FROM smelt.sources.fact f \
                    JOIN smelt.sources.dim d ON f.dim_id = d.id";
        let verdict = fingerprint_projection(sql, "dim");
        let expected: BTreeSet<String> = ["name".to_string()].into_iter().collect();
        assert_eq!(verdict, Projection::Columns(expected), "{verdict:?}");
    }

    #[test]
    fn opaque_function_over_unrelated_source_does_not_force_full_row() {
        // The opaque call reads `f` (fact), not `d` (dim) — dim's own
        // projection is unaffected.
        let sql = "SELECT custom_udf(f.id) AS x, d.tier FROM smelt.sources.fact f \
                    JOIN smelt.sources.dim d ON f.dim_id = d.id";
        let verdict = fingerprint_projection(sql, "dim");
        let expected: BTreeSet<String> = ["tier".to_string()].into_iter().collect();
        assert_eq!(verdict, Projection::Columns(expected), "{verdict:?}");
    }

    #[test]
    fn cte_composed_simple_rename_resolves_through_the_walk() {
        let sql = "WITH d AS (SELECT name, tier FROM smelt.sources.dim) \
                    SELECT f.id, d.name FROM smelt.sources.fact f JOIN d ON f.dim_id = d.name";
        let verdict = fingerprint_projection(sql, "dim");
        let expected: BTreeSet<String> = ["name".to_string()].into_iter().collect();
        assert_eq!(verdict, Projection::Columns(expected), "{verdict:?}");
    }

    #[test]
    fn cte_transform_fails_closed_to_full_row() {
        // `d.combined` is not a simple rename chain at the CTE's own level
        // (it's a computed concatenation), so the outer reference to it
        // cannot resolve a leaf and this classifier does not chase into the
        // CTE alias for an embedded reference either — fail closed.
        let sql = "WITH d AS (SELECT name || tier AS combined FROM smelt.sources.dim) \
                    SELECT f.id, d.combined FROM smelt.sources.fact f JOIN d ON f.dim_id = d.combined";
        let verdict = fingerprint_projection(sql, "dim");
        assert!(verdict.is_full_row(), "{verdict:?}");
    }

    #[test]
    fn two_consumers_of_one_source_derive_distinct_projections() {
        let sql_a = "SELECT f.id, d.name, d.tier FROM smelt.sources.fact f \
                      JOIN smelt.sources.dim d ON f.dim_id = d.id";
        let sql_b = "SELECT f.id, d.tier FROM smelt.sources.fact f \
                      JOIN smelt.sources.dim d ON f.dim_id = d.id";
        let verdict_a = fingerprint_projection(sql_a, "dim");
        let verdict_b = fingerprint_projection(sql_b, "dim");
        assert_ne!(
            verdict_a, verdict_b,
            "consumers must not share one derived projection"
        );
        assert_eq!(
            verdict_b,
            Projection::Columns(["tier".to_string()].into_iter().collect())
        );
    }

    #[test]
    fn set_operation_root_fails_closed_to_full_row() {
        let sql = "SELECT d.name FROM smelt.sources.dim d \
                    UNION ALL \
                    SELECT d2.name FROM smelt.sources.dim d2";
        let verdict = fingerprint_projection(sql, "dim");
        assert!(verdict.is_full_row(), "{verdict:?}");
    }

    // ── projection_identity (F3 sidecar namespace key) ──────────────────

    #[test]
    fn identical_column_projections_share_one_identity() {
        let a = Projection::Columns(
            ["name".to_string(), "tier".to_string()]
                .into_iter()
                .collect(),
        );
        let b = Projection::Columns(
            ["tier".to_string(), "name".to_string()]
                .into_iter()
                .collect(),
        );
        assert_eq!(
            projection_identity(&a),
            projection_identity(&b),
            "column order must not matter — BTreeSet already canonicalizes it"
        );
    }

    #[test]
    fn distinct_column_projections_get_distinct_identities() {
        let a = Projection::Columns(["name".to_string()].into_iter().collect());
        let b = Projection::Columns(["tier".to_string()].into_iter().collect());
        assert_ne!(projection_identity(&a), projection_identity(&b));
    }

    #[test]
    fn full_row_never_shares_an_identity_with_a_columns_projection_of_the_same_names() {
        // A FullRow verdict must get its own distinct identity even when
        // its (hypothetical) column enumeration would coincide with some
        // other consumer's genuinely narrower `Columns` projection — the
        // sidecar section's "never silently widened onto, or narrowed
        // from" rule.
        let full_row = Projection::FullRow {
            reason: "SELECT * over the source".to_string(),
        };
        let columns = Projection::Columns(
            ["name".to_string(), "tier".to_string()]
                .into_iter()
                .collect(),
        );
        assert_ne!(
            projection_identity(&full_row),
            projection_identity(&columns)
        );
    }

    #[test]
    fn full_row_identity_is_stable_regardless_of_reason() {
        let a = Projection::FullRow {
            reason: "SELECT * over the source".to_string(),
        };
        let b = Projection::FullRow {
            reason: "an opaque function call over the source".to_string(),
        };
        assert_eq!(
            projection_identity(&a),
            projection_identity(&b),
            "every fail-closed reason shares one full-row sidecar partition"
        );
    }
}
