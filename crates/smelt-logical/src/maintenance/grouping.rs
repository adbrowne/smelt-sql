//! Per-column mutation-sensitivity / column provenance
//! (`model_properties.md` §"Per-column mutation-sensitivity / column
//! provenance"; `incremental_models.md` §Design "Factoring by
//! mutation-sensitivity"): for each non-skeleton output column, which
//! sources' *post-creation* deltas can change that column's value.
//!
//! **Leaf classifier, not a composition walk.** This derivation resolves
//! provenance over the model's own outermost `SELECT` scope only — its FROM
//! items and each select-item's own expression tree — matching exactly the
//! scope the tracer catalogue (EX-02/07/13/…) already covers. A CTE, set
//! operation, derived-table FROM item, or an unqualified reference ambiguous
//! among more than one joined source is outside what this classifier
//! resolves; per the fail-closed constraint (`incremental_models.md`
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
//!
//! **Membership sensitivity is a second, independent pass.** Alongside the
//! per-select-item value-sensitivity walk above, this module also scans
//! every row-admission position (via the shared `analysis::expr_util`
//! conjunct-splitter and column-ref collector — never an ad hoc text scan)
//! for reads of a `MutableSnapshot` source: a row-admission read can
//! retroactively add or remove rows the model already materialized even
//! when the source's columns never appear in a select item
//! (`docs/specs/model_properties.md` §"Per-column mutation-sensitivity /
//! column provenance", membership paragraph). Two admission positions are
//! covered: a `JOIN`'s `ON` predicate, and a top-level `WHERE`/`HAVING`
//! conjunct (a direct column read of a mutable source — the spec's
//! semi-join case, `WHERE x IN (SELECT ... FROM smelt.sources.<mutable>)`,
//! is handled by failing closed, below, not by resolving into the
//! subquery). The derived membership set attaches to *every* payload group
//! this single-`SELECT` model produces — one admission read governs the
//! whole row set at this scope. An `AppendOnly` source contributes nothing
//! to membership even when read in one of these positions (the
//! retroactive-admission hazard of an append is a distinct, deferred
//! question — `docs/plans/20260808-membership-sensitivity.md` "Explicitly
//! deferred"). Unsupported by this leaf classifier, and reported as a
//! fail-closed whole-model collapse exactly like an unresolvable
//! select-item reference above: a `JOIN` with no resolvable `ON` predicate
//! (e.g. `USING`, an implicit/comma join); an `ON`/`WHERE`/`HAVING` column
//! reference that cannot be resolved to exactly one `FROM` source; or any
//! subquery (`IN`/`EXISTS`/scalar) inside a `WHERE`/`HAVING` conjunct —
//! this classifier never recurses into a nested `SELECT`'s own FROM/aliases.

use std::collections::{BTreeMap, BTreeSet};

use super::{ColumnGroup, MutationProfile, SourceFacts};
use crate::analysis::expr_util::{collect_column_refs, split_top_level_conjuncts};
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
    // does not attempt (tracked: `docs/specs/incremental_models.md` §Known
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

    let membership_sensitivity =
        match membership_sensitivity_sources(&select, &from_clause, &aliases, &source_by_name) {
            Ok(set) => set,
            Err(reason) => {
                return degenerate_whole_model(&payload_columns, &all_source_names, &reason);
            }
        };

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
    // Membership sensitivity is row-scoped, not per-column: one admission
    // read governs every payload group this single-`SELECT` model produces,
    // so the same derived set attaches uniformly to each group below
    // (`docs/specs/model_properties.md` §"Per-column mutation-sensitivity /
    // column provenance", membership paragraph).
    let groups = buckets
        .into_iter()
        .map(|(mutation_sensitivity, columns)| ColumnGroup {
            columns,
            mutation_sensitivity,
            membership_sensitivity: membership_sensitivity.clone(),
        })
        .collect();

    GroupingResult { groups, degenerate }
}

/// Row-admission-position membership sensitivity
/// (`docs/specs/model_properties.md` §"Per-column mutation-sensitivity /
/// column provenance", membership paragraph): every `MutableSnapshot`
/// source read in a position that decides whether an output row exists —
/// a `JOIN`'s `ON` predicate, or a top-level `WHERE`/`HAVING` conjunct
/// (the spec's semi-join case: `WHERE x IN (SELECT ... FROM
/// smelt.sources.<mutable>)` is exactly a semi-join admission read). Scoped
/// to conjuncts this leaf classifier can resolve without recursing into a
/// nested scope: a `JOIN` with no resolvable `ON` predicate (`USING`,
/// natural, or the ANSI-89 implicit comma form), an `ON`/`WHERE`/`HAVING`
/// column reference that cannot be resolved to exactly one `FROM` source,
/// or ANY subquery inside a `WHERE`/`HAVING` conjunct (`IN`/`EXISTS`/scalar
/// — this classifier never resolves into a nested `SELECT`) is outside its
/// resolution and reported as an `Err` so the caller collapses fail-closed
/// rather than silently deriving an empty (falsely-safe) membership set.
/// `Ok` is returned even when there are no joins and no `WHERE`/`HAVING` at
/// all (an empty set — nothing governs row admission beyond the base
/// table).
fn membership_sensitivity_sources(
    select: &smelt_parser::SelectStmt,
    from_clause: &smelt_parser::FromClause,
    aliases: &BTreeMap<String, String>,
    source_by_name: &BTreeMap<&str, &SourceFacts>,
) -> Result<BTreeSet<String>, String> {
    let mut sensitivity = BTreeSet::new();

    // Resolve one conjunct's column references into `sensitivity`,
    // fail-closed on an unresolvable qualifier. `position` names the
    // syntactic position for the error message only.
    let mut resolve_conjunct =
        |conjunct: &smelt_parser::Expr, position: &str| -> Result<(), String> {
            for cref in collect_column_refs(conjunct) {
                let resolved = match cref.qualifier() {
                    Some(q) => aliases.get(&q.to_ascii_lowercase()).cloned(),
                    None if aliases.len() == 1 => aliases.values().next().cloned(),
                    None => None,
                };
                let Some(resolved_path) = resolved else {
                    return Err(format!(
                        "{position} column reference '{}' does not resolve to exactly one \
                     FROM source",
                        cref.name()
                    ));
                };
                let bare = resolved_path
                    .strip_prefix("sources.")
                    .unwrap_or(resolved_path.as_str())
                    .to_string();
                match source_by_name.get(bare.as_str()) {
                    Some(facts) if facts.mutation == MutationProfile::MutableSnapshot => {
                        sensitivity.insert(facts.name.clone());
                    }
                    Some(_) => {
                        // AppendOnly source read in an admission position: the
                        // retroactive-admission hazard of a later-arriving
                        // append is a distinct, deferred question
                        // (`docs/plans/20260808-membership-sensitivity.md`
                        // "Explicitly deferred") — contributes nothing here.
                    }
                    None => {
                        // Unknown provenance: fail closed exactly as the
                        // select-item collector does above.
                        sensitivity.insert(bare);
                    }
                }
            }
            Ok(())
        };

    for join in from_clause.joins() {
        let Some(condition) = join.condition() else {
            return Err(
                "a JOIN has no resolvable ON predicate (USING, natural, or an implicit \
                 comma join) — membership admission cannot be derived for it"
                    .to_string(),
            );
        };
        if !condition.is_on() {
            return Err(
                "a JOIN's condition is a USING clause, not an ON predicate — membership \
                 admission cannot be derived for it"
                    .to_string(),
            );
        }
        let Some(on_expr) = condition.on_expression() else {
            return Err("a JOIN's ON condition has no expression".to_string());
        };
        let mut conjuncts = Vec::new();
        split_top_level_conjuncts(&on_expr, &mut conjuncts);
        for conjunct in &conjuncts {
            resolve_conjunct(conjunct, "ON-predicate")?;
        }
    }

    // WHERE/HAVING: a mutable source read directly in a top-level conjunct
    // is a row-admission read exactly like a JOIN's ON predicate. A
    // subquery (IN/EXISTS/scalar) is a semi-join admission read the spec
    // explicitly names, but this leaf classifier never recurses into a
    // nested SELECT's own FROM/aliases — it fails closed instead of
    // silently treating the predicate as if it read nothing.
    let where_expr = select.where_clause().and_then(|w| w.expression());
    let having_expr = select.having_clause().and_then(|h| h.expression());
    for (clause_expr, position) in [(where_expr, "WHERE"), (having_expr, "HAVING")] {
        let Some(clause_expr) = clause_expr else {
            continue;
        };
        let mut conjuncts = Vec::new();
        split_top_level_conjuncts(&clause_expr, &mut conjuncts);
        for conjunct in &conjuncts {
            if conjunct
                .syntax()
                .descendants()
                .any(|n| n.kind() == smelt_parser::SyntaxKind::SELECT_STMT)
            {
                return Err(format!(
                    "a {position} conjunct contains a subquery (IN/EXISTS/scalar) — \
                     membership admission cannot be derived through it"
                ));
            }
            resolve_conjunct(conjunct, position)?;
        }
    }
    Ok(sensitivity)
}

/// The fail-closed collapse: every payload column lands in one group
/// sensitive to every declared source ("widens to the whole table, never
/// silently" — `incremental_models.md` §"Constraints & Invariants"). Both
/// sensitivity kinds widen to the same "every declared source" set — a
/// degenerate model has no reliable provenance of either kind, so the
/// membership set is set no narrower than value sensitivity (a safe
/// superset: the recompute family membership sensitivity forces is never
/// wrong where a column-scoped merge would also have been admissible).
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
        membership_sensitivity: all_sources.clone(),
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
