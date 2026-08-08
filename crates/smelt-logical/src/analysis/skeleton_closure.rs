//! Skeleton-source-closure proof (P1, `model_properties.md` §"Skeleton-source
//! closure"): decides whether an enrichment join's output rows are entirely
//! accounted for by the driving (fact) side, so the enrichment side can
//! never add, drop, or duplicate a row.
//!
//! Composes five independently-sourced conjuncts, each named at its own
//! failure site:
//! 1. Skeleton-role extraction (`maintenance::skeleton::skeleton_roles`) — no
//!    column that reads the enrichment side occupies a row-membership
//!    position (`Identity`/`Grouping`/`Dedup`/`Ordering`).
//! 2. Per-column provenance (`maintenance::grouping::derive_column_groups`)
//!    — every enrichment-side column's resolved provenance is confined to
//!    the enrichment source alone, never blended with the driving side's.
//! 3. One-to-one join contribution (`analysis::join_shape::fan_out`) — the
//!    enrichment join is proven `OneToOne`, never `OneToMany`.
//! 4. Row preservation — a `LEFT JOIN` always qualifies; an inner/equi-join
//!    qualifies only under the source's declared `referential_integrity`
//!    world-fact (`sources.md` §"Referential integrity").
//! 5. No membership predicate on an enrichment-side column — a leaf-level
//!    scan of the scope's own `WHERE`/`HAVING` predicate AST (never a
//!    substring scan) for any column reference qualified by the enrichment
//!    join's alias.
//!
//! **v1 scope restriction, checked first, not a sixth conjunct**: the
//! enrichment join must sit below any aggregation in the scope — a `GROUP
//! BY` or a window `OVER` in the same scope as the enrichment join changes
//! the skeleton question (which rows survive the fold, not which rows
//! survive the join) and is `Open` regardless of the five conjuncts.
//!
//! Any conjunct that cannot be proven — not just one that is positively
//! disproven — yields `Open`: this is an *and* of five provable facts, not a
//! default with exceptions (`model_properties.md` §"Constraints &
//! Invariants" "Proofs are fail-closed").

use std::collections::BTreeSet;

use smelt_parser::{Expr, FromClause, JoinClause, JoinType};

use crate::analysis::expr_util::collect_column_refs;
use crate::analysis::join_shape::{fan_out, Cardinality, JoinContext};
use crate::analysis::source_bounds::resolve_table_ref_source_name;
use crate::analysis::{item_expr, select_stmt_items};
use crate::maintenance::grouping::derive_column_groups;
use crate::maintenance::skeleton::skeleton_roles;

/// The five-conjunct verdict (`model_properties.md` §"Skeleton-source
/// closure"). `Open` always names which conjunct (or the v1 scope
/// restriction) failed to prove.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkeletonSourceClosure {
    /// All five conjuncts are proven: the enrichment join's output rows are
    /// entirely accounted for by the driving side.
    Closed,
    /// At least one conjunct could not be proven — never a positive
    /// disproof only; absence of proof is refusal.
    Open { reason: String },
}

impl SkeletonSourceClosure {
    pub fn is_closed(&self) -> bool {
        matches!(self, SkeletonSourceClosure::Closed)
    }
}

fn open(reason: impl Into<String>) -> SkeletonSourceClosure {
    SkeletonSourceClosure::Open {
        reason: reason.into(),
    }
}

/// Decide the skeleton-source closure of `sql`'s outermost `SELECT` scope
/// for its join against `enrichment_source` (a `smelt.<path>` source name,
/// `sources.`-prefix optional — matching
/// [`resolve_table_ref_source_name`]'s output convention).
///
/// `referential_integrity` is the enrichment source's declared
/// `referential_integrity:` world-fact (`sources.md`), when any — `None`/
/// empty licenses nothing (conjunct 4 fails closed to `Open` for a
/// non-`LEFT JOIN` enrichment absent a non-empty declaration). `join_ctx`
/// carries the declared unique-key facts [`fan_out`] needs to prove the
/// join `OneToOne` (conjunct 3).
///
/// Fail-closed throughout: an unparseable `sql`, a scope with no top-level
/// `SELECT`, or no join found against `enrichment_source` in that scope's
/// own `FROM`/`JOIN` clauses all yield `Open`.
pub fn skeleton_source_closure(
    sql: &str,
    enrichment_source: &str,
    referential_integrity: Option<&[String]>,
    join_ctx: &JoinContext,
) -> SkeletonSourceClosure {
    let stripped = crate::types::Frontmatter::strip(sql);
    let parse = smelt_parser::parse(stripped);
    let Some(file) = smelt_parser::File::cast(parse.syntax()) else {
        return open("model SQL did not parse to a query this proof can classify");
    };
    let Some(select) = file.select_stmt() else {
        return open("model SQL has no top-level SELECT to classify");
    };

    // v1 scope restriction, checked first: join-below-aggregation is Open
    // regardless of the five conjuncts.
    if select.group_by_clause().is_some() {
        return open(
            "v1 scope restriction: this scope has a GROUP BY above the enrichment join — \
             join-below-aggregation changes the skeleton question and is Open regardless of \
             the five conjuncts",
        );
    }
    if let Some(items) = select_stmt_items(&select) {
        if items
            .iter()
            .any(|item| item_expr(item).window_spec().is_some())
        {
            return open(
                "v1 scope restriction: this scope has a window OVER clause above the \
                 enrichment join — join-below-aggregation changes the skeleton question and \
                 is Open regardless of the five conjuncts",
            );
        }
    }

    let Some(from_clause) = select.from_clause() else {
        return open("model SQL has no FROM clause");
    };

    let Some((join, alias)) = find_enrichment_join(&from_clause, enrichment_source) else {
        return open(format!(
            "no top-level join against '{enrichment_source}' found in this scope's own FROM/JOIN \
             clauses"
        ));
    };

    // Conjunct 3: one-to-one join contribution.
    if fan_out(&join, join_ctx) != Cardinality::OneToOne {
        return open(format!(
            "one-to-one join contribution: the join against '{enrichment_source}' is not \
             proven OneToOne (fan-out/cardinality, `analysis::join_shape::fan_out`)"
        ));
    }

    // Conjunct 4: row preservation.
    let is_outer_preserving = matches!(join.join_type(), Some(JoinType::Left));
    if !is_outer_preserving {
        let ri_licenses = referential_integrity.is_some_and(|ri| !ri.is_empty());
        if !ri_licenses {
            return open(format!(
                "row preservation: the join against '{enrichment_source}' is an inner/equi-join \
                 with no declared referential_integrity world-fact — only a LEFT JOIN or a \
                 declared referential_integrity proves every driving row survives the join"
            ));
        }
    }

    // Conjunct 1: skeleton-role extraction — no enrichment-side column
    // occupies a row-membership position. `declared_unique_key`/
    // `partition_col` are the *output's own* row-identity facts, orthogonal
    // to this join's closure, so both are passed empty here: only the
    // scope's own GROUP BY/DISTINCT/ORDER BY positions are consulted (and
    // GROUP BY is already excluded by the v1 restriction above).
    let Some(roles) = skeleton_roles(sql, &[], None) else {
        return open("skeleton-role extraction did not classify this model's SELECT scope");
    };
    let Some(items) = select_stmt_items(&select) else {
        return open("the SELECT list did not classify (e.g. a wildcard item)");
    };
    for (item, (_alias, role)) in items.iter().zip(roles.iter()) {
        if role.is_skeleton() && expr_references_alias(item_expr(item), &alias) {
            return open(format!(
                "skeleton-role extraction: an enrichment-side column occupies a row-membership \
                 position ({role:?}) — no skeleton column may originate on the enrichment side"
            ));
        }
    }

    // Conjunct 2: per-column provenance confined to its own source. An
    // empty `sources` slice makes every resolved column reference fall to
    // `derive_column_groups`'s fail-closed "unknown provenance" branch,
    // which inserts the referenced source's own bare name — exactly the
    // raw FROM-resolution fact this conjunct needs, with no dependency on
    // real mutation-profile facts this proof does not have.
    let grouping = derive_column_groups(sql, &[], &BTreeSet::new());
    if !grouping.degenerate.is_empty() {
        return open(format!(
            "per-column provenance: {} column(s) had unresolvable provenance ({})",
            grouping.degenerate.len(),
            grouping
                .degenerate
                .iter()
                .map(|d| d.column.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    for group in &grouping.groups {
        if group.mutation_sensitivity.contains(enrichment_source)
            && group.mutation_sensitivity.len() > 1
        {
            return open(format!(
                "per-column provenance: column(s) {:?} blend '{enrichment_source}' with another \
                 source's provenance — not confined to the enrichment source alone",
                group.columns
            ));
        }
    }

    // Conjunct 5: no membership predicate on an enrichment-side column.
    if let Some(where_clause) = select.where_clause() {
        if let Some(expr) = where_clause.expression() {
            if expr_references_alias(&expr, &alias) {
                return open(format!(
                    "no membership predicate: the WHERE clause tests an enrichment-side column \
                     (alias '{alias}') — output row-membership would depend on the enrichment \
                     source, breaking the closure"
                ));
            }
        }
    }
    if let Some(having_clause) = select.having_clause() {
        if let Some(expr) = having_clause.expression() {
            if expr_references_alias(&expr, &alias) {
                return open(format!(
                    "no membership predicate: the HAVING clause tests an enrichment-side column \
                     (alias '{alias}') — output row-membership would depend on the enrichment \
                     source, breaking the closure"
                ));
            }
        }
    }

    SkeletonSourceClosure::Closed
}

/// Resolve the alias (or bare identifier, when unaliased) that `sql`'s
/// outermost query scope qualifies `enrichment_source`'s columns with, if it
/// is joined at all in that scope's own `FROM`/`JOIN` clauses. `None` when
/// `sql` doesn't parse, has no top-level `SELECT`/`FROM`, or
/// `enrichment_source` is not the target of a join there (e.g. it is itself
/// the `FROM`-clause driving table, or is not referenced in this scope at
/// all).
///
/// Used by `maintenance::derive::append_model_edge_cells` (T3, `docs/plans/
/// 20260715-composed-axes-conditional-maintenance.md` Phase E3) to build the
/// [`JoinContext`] the one-to-one conjunct (3) needs from a model edge's own
/// declared `unique_key`, without duplicating this module's private
/// join-finding logic in a second parse of the same SQL.
pub(crate) fn enrichment_join_alias(sql: &str, enrichment_source: &str) -> Option<String> {
    let stripped = crate::types::Frontmatter::strip(sql);
    let parse = smelt_parser::parse(stripped);
    let file = smelt_parser::File::cast(parse.syntax())?;
    let select = file.select_stmt()?;
    let from_clause = select.from_clause()?;
    find_enrichment_join(&from_clause, enrichment_source).map(|(_, alias)| alias)
}

/// Find the top-level join in `from_clause` whose `smelt.<path>` table ref
/// resolves to `enrichment_source` (`sources.`-prefix optional, matching
/// [`resolve_table_ref_source_name`]'s convention), plus the alias (or bare
/// identifier, when unaliased) the join condition and the rest of the scope
/// qualify its columns with.
fn find_enrichment_join(
    from_clause: &FromClause,
    enrichment_source: &str,
) -> Option<(JoinClause, String)> {
    for join in from_clause.joins() {
        let table_ref = join.table_ref()?;
        let resolved = resolve_table_ref_source_name(&table_ref)?;
        let bare = resolved
            .strip_prefix("sources.")
            .unwrap_or(resolved.as_str());
        if bare == enrichment_source || resolved == enrichment_source {
            let alias = table_ref.alias().or_else(|| table_ref.identifier())?;
            return Some((join, alias));
        }
    }
    None
}

/// Whether `expr`'s syntax tree contains any column reference qualified by
/// `alias` — a leaf-level AST walk over one already-parsed expression's own
/// syntax tree (never a raw-text scan), shared by conjuncts 1 and 5.
fn expr_references_alias(expr: &Expr, alias: &str) -> bool {
    collect_column_refs(expr).iter().any(|cref| {
        cref.qualifier()
            .is_some_and(|q| q.eq_ignore_ascii_case(alias))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn left_join_payload_only_closes() {
        let sql = "SELECT f.id, f.amount, d.category FROM smelt.sources.fact f \
                    LEFT JOIN smelt.sources.dim d ON f.dim_id = d.id";
        let ctx = JoinContext::new().with_unique_key("d", "id");
        let verdict = skeleton_source_closure(sql, "dim", None, &ctx);
        assert_eq!(verdict, SkeletonSourceClosure::Closed, "{verdict:?}");
    }

    #[test]
    fn inner_join_with_referential_integrity_closes() {
        let sql = "SELECT f.id, f.amount, d.category FROM smelt.sources.fact f \
                    JOIN smelt.sources.dim d ON f.dim_id = d.id";
        let ctx = JoinContext::new().with_unique_key("d", "id");
        let ri = vec!["id".to_string()];
        let verdict = skeleton_source_closure(sql, "dim", Some(&ri), &ctx);
        assert_eq!(verdict, SkeletonSourceClosure::Closed, "{verdict:?}");
    }

    #[test]
    fn bare_inner_join_without_referential_integrity_stays_open() {
        let sql = "SELECT f.id, f.amount, d.category FROM smelt.sources.fact f \
                    JOIN smelt.sources.dim d ON f.dim_id = d.id";
        let ctx = JoinContext::new().with_unique_key("d", "id");
        let verdict = skeleton_source_closure(sql, "dim", None, &ctx);
        assert!(!verdict.is_closed(), "{verdict:?}");
    }

    #[test]
    fn membership_predicate_on_enrichment_column_stays_open() {
        let sql = "SELECT f.id, f.amount, d.category FROM smelt.sources.fact f \
                    LEFT JOIN smelt.sources.dim d ON f.dim_id = d.id \
                    WHERE d.category = 'gold'";
        let ctx = JoinContext::new().with_unique_key("d", "id");
        let verdict = skeleton_source_closure(sql, "dim", None, &ctx);
        assert!(!verdict.is_closed(), "{verdict:?}");
        assert!(
            matches!(verdict, SkeletonSourceClosure::Open { reason } if reason.contains("membership predicate"))
        );
    }

    #[test]
    fn enrichment_column_feeding_group_by_stays_open() {
        let sql = "SELECT d.category, COUNT(*) AS n FROM smelt.sources.fact f \
                    LEFT JOIN smelt.sources.dim d ON f.dim_id = d.id \
                    GROUP BY d.category";
        let ctx = JoinContext::new().with_unique_key("d", "id");
        let verdict = skeleton_source_closure(sql, "dim", None, &ctx);
        assert!(!verdict.is_closed(), "{verdict:?}");
        assert!(
            matches!(verdict, SkeletonSourceClosure::Open { reason } if reason.contains("v1 scope restriction"))
        );
    }

    #[test]
    fn fan_out_join_stays_open() {
        let sql = "SELECT f.id, f.amount, d.category FROM smelt.sources.fact f \
                    LEFT JOIN smelt.sources.dim d ON f.dim_id = d.category";
        // No declared unique key covers the join's equality columns.
        let ctx = JoinContext::new().with_unique_key("d", "id");
        let verdict = skeleton_source_closure(sql, "dim", None, &ctx);
        assert!(!verdict.is_closed(), "{verdict:?}");
        assert!(
            matches!(verdict, SkeletonSourceClosure::Open { reason } if reason.contains("one-to-one"))
        );
    }

    #[test]
    fn no_join_against_enrichment_source_stays_open() {
        let sql = "SELECT f.id FROM smelt.sources.fact f";
        let ctx = JoinContext::new();
        let verdict = skeleton_source_closure(sql, "dim", None, &ctx);
        assert!(!verdict.is_closed(), "{verdict:?}");
    }
}
