//! Per-column mutation-sensitivity / column provenance
//! (`model_properties.md` §"Per-column mutation-sensitivity / column
//! provenance"; `maintenance_plan.md` §Design "Factoring by
//! mutation-sensitivity"): for each non-skeleton output column, which
//! sources' *post-creation* deltas can change that column's value.
//!
//! **Leaf classifier, not a composition walk.** This derivation resolves
//! provenance over the model's own outermost `SELECT` scope only — its FROM
//! items and each select-item's own expression tree — matching exactly the
//! scope the tracer catalogue (EX-02/07/13/…) already covers. A CTE, set
//! operation, derived-table FROM item, or an unqualified reference ambiguous
//! among more than one joined source is outside what this classifier
//! resolves; per the fail-closed constraint (`maintenance_plan.md`
//! §"Constraints & Invariants"), such a model **never** silently drops a
//! source from a column's sensitivity — it collapses the *whole model's*
//! non-skeleton columns into one degenerate group sensitive to every
//! declared source ("the whole table"), and names every column and reason
//! this collapse touches in [`GroupingResult::degenerate`] so the caller
//! sees it, never inherits a quietly narrower plan.
//!
//! **The load-bearing rule.** A column that reads an append-only source
//! *without aggregating over it* is immutable at creation: the source can
//! only ever add rows, never revise the one this column already read, so
//! the reference contributes no sensitivity. The same source read *inside*
//! an aggregate does contribute — a still-open output row can still receive
//! more of that source's rows later, which is itself a post-creation delta
//! of the aggregate's value. A mutable-snapshot source always contributes,
//! aggregated or not, because its already-read row can change in place.

use std::collections::{BTreeMap, BTreeSet};

use smelt_parser::syntax_kind::SyntaxNode;
use smelt_parser::{ColumnRef, Expr};

use super::{ColumnGroup, MutationProfile, SourceFacts};
use crate::analysis::source_bounds::resolve_table_ref_source_name;
use crate::analysis::{item_alias, item_expr, select_stmt_items, SelectItemKind};

/// One column the derivation could not resolve cleanly — surfaced,
/// never silently dropped from the collapsed group's provenance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DegenerateColumn {
    pub column: String,
    pub reason: String,
}

/// The derived column-group partition, plus every degenerate collapse the
/// derivation had to make.
#[derive(Debug, Clone, Default)]
pub struct GroupingResult {
    pub groups: Vec<ColumnGroup>,
    pub degenerate: Vec<DegenerateColumn>,
}

/// Derive [`ColumnGroup`]s for `sql`'s non-skeleton output columns from
/// column provenance × each source's declared [`MutationProfile`].
/// `skeleton_columns` are excluded — creation is shared by every column, so
/// the mutation-sensitivity axis only partitions the payload.
pub fn derive_column_groups(
    sql: &str,
    sources: &[SourceFacts],
    skeleton_columns: &BTreeSet<String>,
) -> GroupingResult {
    let all_source_names: BTreeSet<String> = sources.iter().map(|s| s.name.clone()).collect();
    let stripped = crate::types::Frontmatter::strip(sql);
    let parse = smelt_parser::parse(stripped);
    let Some(file) = smelt_parser::File::cast(parse.syntax()) else {
        return GroupingResult::default();
    };
    let Some(select) = file.select_stmt() else {
        return GroupingResult::default();
    };

    let Some(items) = select_stmt_items(&select) else {
        // A wildcard item or another unclassifiable select-list shape: no
        // individual column name is even known here, so the collapse is
        // reported against the whole SELECT list rather than silently
        // returning "no groups, no degenerate columns" (which would read as
        // "nothing mutation-sensitive here", the opposite of the truth).
        return GroupingResult {
            groups: Vec::new(),
            degenerate: vec![DegenerateColumn {
                column: "*".to_string(),
                reason: "the SELECT list did not classify (e.g. a wildcard item) — no \
                         per-column provenance could be derived"
                    .to_string(),
            }],
        };
    };
    let payload_columns: Vec<String> = items
        .iter()
        .map(|i| item_alias(i).to_string())
        .filter(|c| !skeleton_columns.contains(c))
        .collect();

    // v0 shape guard: this classifier resolves provenance only through a
    // single top-level SELECT scope's own FROM items — the shape every
    // tracer catalogue example uses. A CTE or set operation composes
    // provenance through more than one scope, which this leaf classifier
    // does not attempt (tracked: `docs/specs/maintenance_plan.md` §Known
    // Divergences).
    if select.with_clause().is_some() || select.has_set_operation() {
        return degenerate_whole_model(
            &payload_columns,
            &all_source_names,
            "model SQL composes through a CTE or set operation, outside this leaf \
             classifier's single-scope provenance resolution",
        );
    }

    let Some(from_clause) = select.from_clause() else {
        return GroupingResult::default();
    };

    let mut aliases: BTreeMap<String, String> = BTreeMap::new();
    let mut unsupported_from: Option<String> = None;
    for table_ref in from_clause
        .table_refs()
        .chain(from_clause.joins().filter_map(|j| j.table_ref()))
    {
        if table_ref.subquery().is_some() {
            unsupported_from = Some("a FROM item is a derived table (subquery)".to_string());
            continue;
        }
        let Some(resolved) = resolve_table_ref_source_name(&table_ref) else {
            unsupported_from =
                Some("a FROM item is not a resolvable `smelt.<path>` source reference".to_string());
            continue;
        };
        let key = table_ref
            .alias()
            .unwrap_or_else(|| resolved.clone())
            .to_ascii_lowercase();
        aliases.insert(key, resolved);
    }
    if let Some(reason) = unsupported_from {
        return degenerate_whole_model(&payload_columns, &all_source_names, &reason);
    }

    let source_by_name: BTreeMap<&str, &SourceFacts> =
        sources.iter().map(|s| (s.name.as_str(), s)).collect();

    let mut per_column: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut degenerate = Vec::new();
    let mut collapse = false;

    for item in &items {
        let alias = item_alias(item).to_string();
        if skeleton_columns.contains(&alias) {
            continue;
        }
        let is_aggregate = matches!(
            item,
            SelectItemKind::OtherAggregate { .. } | SelectItemKind::CountDistinct { .. }
        );
        let refs = collect_column_refs(item_expr(item));
        let mut sensitivity = BTreeSet::new();
        let mut column_reason = None;
        for cref in &refs {
            let resolved = match cref.qualifier() {
                Some(q) => aliases.get(&q.to_ascii_lowercase()).cloned(),
                None if aliases.len() == 1 => aliases.values().next().cloned(),
                None => None,
            };
            let Some(resolved_path) = resolved else {
                column_reason = Some(format!(
                    "column reference '{}' does not resolve to exactly one FROM source",
                    cref.name()
                ));
                break;
            };
            let bare = resolved_path
                .strip_prefix("sources.")
                .unwrap_or(resolved_path.as_str())
                .to_string();
            match source_by_name.get(bare.as_str()) {
                Some(facts) => {
                    let contributes = match facts.mutation {
                        MutationProfile::MutableSnapshot => true,
                        MutationProfile::AppendOnly => is_aggregate,
                    };
                    if contributes {
                        sensitivity.insert(facts.name.clone());
                    }
                }
                None => {
                    // Unknown provenance (e.g. a model reference this v0
                    // classifier does not carry mutation facts for): fail
                    // closed by assuming it *does* contribute, rather than
                    // silently treating an unrecognised source as safe.
                    sensitivity.insert(bare);
                }
            }
        }
        if let Some(reason) = column_reason {
            degenerate.push(DegenerateColumn {
                column: alias,
                reason,
            });
            collapse = true;
            continue;
        }
        per_column.insert(alias, sensitivity);
    }

    if collapse {
        let mut result = degenerate_whole_model(
            &payload_columns,
            &all_source_names,
            "one or more columns had unresolvable provenance",
        );
        // The collapse reason above is the model-wide summary; keep the
        // specific per-column reasons too, so the surfaced report names
        // exactly which reference could not be resolved and why.
        result.degenerate = degenerate;
        return result;
    }

    let mut buckets: BTreeMap<BTreeSet<String>, Vec<String>> = BTreeMap::new();
    for (col, sens) in per_column {
        buckets.entry(sens).or_default().push(col);
    }
    let groups = buckets
        .into_iter()
        .map(|(mutation_sensitivity, columns)| ColumnGroup {
            columns,
            mutation_sensitivity,
        })
        .collect();

    GroupingResult { groups, degenerate }
}

/// The fail-closed collapse: every payload column lands in one group
/// sensitive to every declared source ("widens to the whole table, never
/// silently" — `maintenance_plan.md` §"Constraints & Invariants").
fn degenerate_whole_model(
    payload_columns: &[String],
    all_sources: &BTreeSet<String>,
    reason: &str,
) -> GroupingResult {
    if payload_columns.is_empty() {
        return GroupingResult::default();
    }
    let group = ColumnGroup {
        columns: payload_columns.to_vec(),
        mutation_sensitivity: all_sources.clone(),
    };
    let degenerate = payload_columns
        .iter()
        .map(|c| DegenerateColumn {
            column: c.clone(),
            reason: reason.to_string(),
        })
        .collect();
    GroupingResult {
        groups: vec![group],
        degenerate,
    }
}

/// Recursively collect every simple (possibly qualified) column reference
/// inside `expr` — a leaf classifier over one already-parsed select-item
/// expression's own syntax tree (never the whole model text).
fn collect_column_refs(expr: &Expr) -> Vec<ColumnRef> {
    let mut out = Vec::new();
    collect_column_refs_rec(expr.syntax(), &mut out);
    out
}

fn collect_column_refs_rec(node: &SyntaxNode, out: &mut Vec<ColumnRef>) {
    if let Some(e) = Expr::cast(node.clone()) {
        if let Some(cref) = ColumnRef::from_expr(&e) {
            out.push(cref);
            return;
        }
    }
    for child in node.children() {
        collect_column_refs_rec(&child, out);
    }
}
