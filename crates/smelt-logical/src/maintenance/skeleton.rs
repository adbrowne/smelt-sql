//! Skeleton-role extraction (`model_properties.md` §"Skeleton-role
//! extraction"): classifies each of a model's own output columns as
//! occupying a row-membership/identity position (`Identity` / `Grouping` /
//! `Dedup` / `Ordering`) or a plain `Payload` position.
//!
//! **Leaf classifier, not a composition walk.** This derivation reads only
//! the model's own outermost `SELECT` scope — its own `GROUP BY`, `DISTINCT`,
//! and `ORDER BY` clauses, resolved against its own select-list items — plus
//! the caller-supplied declared unique key (a world-fact, never derived).
//! It does not compose through CTEs, derived tables, or set-operation
//! branches; a model shaped that way is outside v0's supported scope and the
//! caller falls back to a hand-declared `skeleton_columns` set, exactly as
//! today's tracer already does for every model shape it cannot classify
//! (`docs/specs/maintenance_plan.md` §Known Divergences).
//!
//! Fail-closed by construction: a column this classifier cannot place in
//! `Identity`/`Grouping`/`Dedup`/`Ordering` is `Payload` only when the SQL
//! positively shows it is not a row-membership key; when the SQL cannot be
//! parsed at all, nothing is classified and the caller's declared
//! `unique_key` is the only skeleton set returned — never an empty guess.

use std::collections::BTreeSet;

use crate::analysis::{item_alias, item_expr, resolve_scope_group_by, select_stmt_items};

/// The row-membership/identity position a column occupies, or `Payload`
/// (`model_properties.md` §"Skeleton-role extraction").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkeletonRole {
    /// A declared unique-key member.
    Identity,
    /// A `GROUP BY` key of the model's own outermost scope.
    Grouping,
    /// A `SELECT DISTINCT` dedup key (the whole projected row).
    Dedup,
    /// An `ORDER BY` key of the model's own outermost scope.
    Ordering,
    /// Not a row-membership position.
    Payload,
}

impl SkeletonRole {
    pub fn is_skeleton(&self) -> bool {
        !matches!(self, SkeletonRole::Payload)
    }
}

/// Classify every projected column of `sql`'s outermost `SELECT` scope.
/// `declared_unique_key` is the model's own declared unique key (a
/// world-fact); its members are always `Identity` regardless of what the
/// query shape otherwise proves. `partition_col`, when the output grain is
/// partition-addressed (`Grain::Partition`), is likewise always `Identity` —
/// it is the address every maintenance write targets, a row-membership fact
/// independent of whether the SQL also happens to `GROUP BY` it. Returns
/// `None` when `sql` has no top-level `SELECT` to classify (fail-closed: the
/// caller must not treat this as "no skeleton columns").
pub fn skeleton_roles(
    sql: &str,
    declared_unique_key: &[String],
    partition_col: Option<&str>,
) -> Option<Vec<(String, SkeletonRole)>> {
    let stripped = crate::types::Frontmatter::strip(sql);
    let parse = smelt_parser::parse(stripped);
    let file = smelt_parser::File::cast(parse.syntax())?;
    let select = file.select_stmt()?;
    let items = select_stmt_items(&select)?;

    let mut unique_key: BTreeSet<String> = declared_unique_key
        .iter()
        .map(|c| c.to_ascii_lowercase())
        .collect();
    if let Some(col) = partition_col {
        unique_key.insert(col.to_ascii_lowercase());
    }
    let group_by_exprs: BTreeSet<String> = resolve_scope_group_by(&select, &items)
        .into_iter()
        .collect();
    let order_by_exprs = resolve_order_by(&select, &items);
    let is_distinct = select.is_distinct();

    let roles = items
        .iter()
        .map(|item| {
            let alias = item_alias(item).to_string();
            let expr_text = item_expr(item).text().trim().to_string();
            let role = if unique_key.contains(&alias.to_ascii_lowercase()) {
                SkeletonRole::Identity
            } else if group_by_exprs.contains(&expr_text) {
                SkeletonRole::Grouping
            } else if is_distinct {
                SkeletonRole::Dedup
            } else if order_by_exprs.contains(&expr_text) {
                SkeletonRole::Ordering
            } else {
                SkeletonRole::Payload
            };
            (alias, role)
        })
        .collect();
    Some(roles)
}

/// Convenience for [`super::OutputSpec::skeleton_columns`]: the set of
/// output columns whose role is not `Payload`, unioned with
/// `declared_unique_key` and `partition_col` (so a key member the SQL
/// doesn't itself project as a `GROUP BY`/`DISTINCT`/`ORDER BY` column is
/// still named skeleton). Fail-closed on an unclassifiable `sql`: returns
/// just the declared key/partition column rather than an empty set, so a
/// caller that forgets to check for `None` still gets the one skeleton fact
/// it is certain of.
pub fn skeleton_columns(
    sql: &str,
    declared_unique_key: &[String],
    partition_col: Option<&str>,
) -> BTreeSet<String> {
    let mut out: BTreeSet<String> = declared_unique_key.iter().cloned().collect();
    out.extend(partition_col.map(|c| c.to_string()));
    if let Some(roles) = skeleton_roles(sql, declared_unique_key, partition_col) {
        out.extend(
            roles
                .into_iter()
                .filter(|(_, role)| role.is_skeleton())
                .map(|(col, _)| col),
        );
    }
    out
}

/// Resolve `ORDER BY` expressions against `select`'s own select-list
/// `items` — ordinal references (`ORDER BY 1`) resolve to this scope's own
/// projections, mirroring [`resolve_scope_group_by`]'s ordinal handling.
fn resolve_order_by(
    select: &smelt_parser::SelectStmt,
    items: &[crate::analysis::SelectItemKind],
) -> BTreeSet<String> {
    let Some(order_by) = select.order_by_clause() else {
        return BTreeSet::new();
    };
    order_by
        .items()
        .filter_map(|oi| oi.expression())
        .map(|expr| {
            let text = expr.text().trim().to_string();
            if let Ok(ordinal) = text.parse::<usize>() {
                if ordinal >= 1 {
                    if let Some(item) = items.get(ordinal - 1) {
                        return item_expr(item).text().trim().to_string();
                    }
                }
            }
            text
        })
        .collect()
}
