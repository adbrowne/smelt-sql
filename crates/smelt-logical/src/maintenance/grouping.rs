//! Per-column mutation-sensitivity / column provenance
//! (`model_properties.md` §"Per-column mutation-sensitivity / column
//! provenance"; `incremental_models.md` §Design "Factoring by
//! mutation-sensitivity"): for each non-skeleton output column, which
//! sources' *post-creation* deltas can change that column's value.
//!
//! **Value sensitivity is walk-composed.** This derivation builds a
//! [`crate::analysis::walk::QueryTree`] over the model's SQL and resolves
//! every select-item's embedded column references via
//! [`crate::analysis::walk::resolve_reference_leaf`] — the same top-scope
//! lineage chase `analysis::fingerprint`'s P4 projection reuses. A simple
//! rename and a reference embedded inside a computed/aggregate expression
//! are resolved identically: both are collected column references, each
//! independently chased through arbitrarily nested single-use CTEs and
//! derived tables to a base-relation column. This is a **leaf classifier
//! invoked by the walk's own lineage chase**, never a raw-text or
//! single-scope-only scan: `resolve_reference_leaf` walks the whole tree's
//! CTE/derived-table structure to build the lineage, this module only reads
//! the already-chased result. Fail-closed exactly as before: an
//! unresolvable reference (an ambiguous unqualified reference, a self-join,
//! a qualifier that does not resolve to any FROM-tree alias, a chase the
//! walk itself could not normalize) collapses the *whole model's*
//! non-skeleton columns into one degenerate group sensitive to every
//! declared source ("the whole table"), naming every column and reason this
//! collapse touches in [`GroupingResult::degenerate`].
//!
//! **The load-bearing rule.** A column that reads an append-only source
//! *without aggregating over it* is immutable at creation: the source can
//! only ever add rows, never revise the one this column already read, so
//! the reference contributes no sensitivity. The same source read *inside*
//! an aggregate does contribute — a still-open output row can still receive
//! more of that source's rows later, which is itself a post-creation delta
//! of the aggregate's value. A mutable-snapshot source always contributes,
//! aggregated or not, because its already-read row can change in place.
//! This rule is applied **per attribution**, not per syntactic position: a
//! leaf reached through a select item's own simple rename chain is a
//! non-aggregated read of that leaf regardless of how many CTE/derived-table
//! hops the chase passed through, and a leaf reached from a reference
//! embedded inside an aggregate function call is an aggregated read — the
//! `is_aggregate` flag is decided once, from the select item's own
//! [`SelectItemKind`] classification, and applied uniformly to every
//! reference the item's expression collects (matching the pre-walk
//! behaviour exactly).
//!
//! **Membership sensitivity is a second, independent pass — still
//! outer-scope this phase.** Alongside the per-select-item value-sensitivity
//! walk above, this module also scans the model's own **outermost**
//! `SELECT` scope's row-admission positions (its own `JOIN` `ON`
//! predicates and top-level `WHERE`/`HAVING` conjuncts, via the shared
//! `analysis::expr_util` conjunct-splitter and column-ref collector — never
//! an ad hoc text scan) for reads of a `MutableSnapshot` source, resolving
//! each reference the same way the value pass does
//! (`resolve_reference_leaf`) — so a top-level `ON`/`WHERE`/`HAVING`
//! conjunct that names a CTE or derived-table alias still resolves to that
//! alias's own base-relation provenance when the alias is a simple rename
//! chain. **This pass does not itself descend into a nested scope's own
//! `WHERE`/`HAVING`/`JOIN` clauses** — a CTE body's or derived table's own
//! admission-relevant predicates are invisible to it, exactly as before
//! this phase (extending the membership scan itself to every scope the walk
//! enumerates is Phase 3's job, `docs/plans/20260809-sensitivity-precision.md`).
//!
//! **The membership-safety boundary this phase enforces.** Because value
//! sensitivity can now see through a CTE/derived-table wrapper while
//! membership sensitivity still cannot see *inside* one, a model whose
//! non-top-level scope hides its own admission-relevant structure — a JOIN,
//! or a `WHERE`/`HAVING` clause, inside a CTE body or a derived table — must
//! not be allowed to produce a nondegenerate plan: doing so would let a
//! mutable join partner buried in that hidden scope silently vanish from
//! every group's membership sensitivity, the exact silent-hole hazard the
//! fail-closed constraint (`incremental_models.md` §"Constraints &
//! Invariants") forbids. [`has_hidden_admission_structure`] checks for
//! exactly this before either pass runs, forcing the same whole-model
//! collapse as an unsupported construct when it finds one. A CTE/derived
//! table that is a **pure projection** over a single source (no JOIN, no
//! `WHERE`/`HAVING` of its own) is not flagged — its own row set is exactly
//! its one source's row set, so there is no hidden admission decision to
//! miss, and the enclosing scope's own JOIN/`WHERE`/`HAVING` (visible to the
//! outer-scope membership pass) governs admission as usual.

use std::collections::{BTreeMap, BTreeSet};

use super::{ColumnGroup, MutationProfile, SourceFacts};
use crate::analysis::expr_util::{collect_column_refs, split_top_level_conjuncts};
use crate::analysis::walk::{resolve_reference_leaf, InputItem, QueryNode, QueryTree};
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

    let tree = QueryTree::from_select(&select);

    if tree.root.has_unsupported() {
        return degenerate_whole_model(
            &payload_columns,
            &all_source_names,
            "model SQL contains a construct the composition walk cannot normalize",
        );
    }
    if matches!(tree.root, QueryNode::SetOp(_)) {
        return degenerate_whole_model(
            &payload_columns,
            &all_source_names,
            "model SQL's outermost query is a set operation — mutation-sensitivity is not \
             yet classified across set-operation arms",
        );
    }
    if has_hidden_admission_structure(&tree.root, true) {
        return degenerate_whole_model(
            &payload_columns,
            &all_source_names,
            "a nested CTE or derived-table scope contains its own JOIN or WHERE/HAVING \
             admission predicate the outer-scope membership pass cannot see (Phase 3, \
             `docs/plans/20260809-sensitivity-precision.md`, extends the membership scan \
             to every scope the walk enumerates)",
        );
    }

    let source_by_name: BTreeMap<&str, &SourceFacts> =
        sources.iter().map(|s| (s.name.as_str(), s)).collect();

    let Some(from_clause) = select.from_clause() else {
        return GroupingResult::default();
    };

    let membership_sensitivity =
        match membership_sensitivity_sources(&tree, &select, &from_clause, &source_by_name) {
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
            let Some(leaf) = resolve_reference_leaf(&tree, cref.qualifier(), cref.name()) else {
                column_reason = Some(format!(
                    "column reference '{}' does not resolve to a base-relation column the \
                     composition walk can chase (an ambiguous unqualified reference, a \
                     self-join, or a reference this walk cannot normalize)",
                    cref.name()
                ));
                break;
            };
            let bare = leaf
                .relation
                .strip_prefix("sources.")
                .unwrap_or(leaf.relation.as_str())
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
    // read governs every payload group this model's outermost scope
    // produces, so the same derived set attaches uniformly to each group
    // below (`docs/specs/model_properties.md` §"Per-column
    // mutation-sensitivity / column provenance", membership paragraph).
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

/// The membership-safety boundary (module doc, "The membership-safety
/// boundary this phase enforces"): `true` when some scope reachable from
/// `node` **other than the top-level scope itself** (`is_top` is only
/// `true` for the very first call) has more than one FROM input (a JOIN) or
/// its own `WHERE`/`HAVING` clause — admission-relevant structure the
/// outer-scope-only membership pass (`membership_sensitivity_sources`)
/// cannot see. `QueryNode::Unsupported` is not re-checked here — the caller
/// already rejects `has_unsupported()` trees before this runs.
fn has_hidden_admission_structure(node: &QueryNode, is_top: bool) -> bool {
    match node {
        QueryNode::Unsupported { .. } => false,
        QueryNode::Select(sn) => {
            let risky_here = !is_top
                && (sn.inputs.len() > 1
                    || sn.select.where_clause().is_some()
                    || sn.select.having_clause().is_some());
            if risky_here {
                return true;
            }
            if sn
                .ctes
                .iter()
                .any(|c| has_hidden_admission_structure(&c.body, false))
            {
                return true;
            }
            sn.inputs.iter().any(|input| match input {
                InputItem::Derived { body, .. } => has_hidden_admission_structure(body, false),
                InputItem::Table { .. }
                | InputItem::CteRef { .. }
                | InputItem::Unsupported { .. } => false,
            })
        }
        QueryNode::SetOp(so) => {
            so.ctes
                .iter()
                .any(|c| has_hidden_admission_structure(&c.body, false))
                || so
                    .branches
                    .iter()
                    .any(|b| has_hidden_admission_structure(b, false))
        }
    }
}

/// Row-admission-position membership sensitivity
/// (`docs/specs/model_properties.md` §"Per-column mutation-sensitivity /
/// column provenance", membership paragraph): every `MutableSnapshot`
/// source read in a position that decides whether an output row exists —
/// a `JOIN`'s `ON` predicate, or a top-level `WHERE`/`HAVING` conjunct
/// (the spec's semi-join case: `WHERE x IN (SELECT ... FROM
/// smelt.sources.<mutable>)` is exactly a semi-join admission read). Column
/// references resolve via `resolve_reference_leaf` — the same walk-composed
/// chase the value pass uses — so a conjunct naming a CTE/derived-table
/// alias that is itself a simple rename chain still resolves to its
/// base-relation provenance (module doc). Scoped to conjuncts of the
/// model's own **outermost** scope: a `JOIN` with no resolvable `ON`
/// predicate (`USING`, natural, or the ANSI-89 implicit comma form), an
/// `ON`/`WHERE`/`HAVING` column reference that cannot be resolved
/// (ambiguous, self-joined, or reaching a shape the walk cannot chase), or
/// ANY subquery inside a `WHERE`/`HAVING` conjunct (`IN`/`EXISTS`/scalar —
/// this pass never resolves into a nested `SELECT`'s own FROM/aliases) is
/// outside its resolution and reported as an `Err` so the caller collapses
/// fail-closed rather than silently deriving an empty (falsely-safe)
/// membership set. `Ok` is returned even when there are no joins and no
/// `WHERE`/`HAVING` at all (an empty set — nothing governs row admission
/// beyond the base table).
fn membership_sensitivity_sources(
    tree: &QueryTree,
    select: &smelt_parser::SelectStmt,
    from_clause: &smelt_parser::FromClause,
    source_by_name: &BTreeMap<&str, &SourceFacts>,
) -> Result<BTreeSet<String>, String> {
    let mut sensitivity = BTreeSet::new();

    // Resolve one conjunct's column references into `sensitivity`,
    // fail-closed on an unresolvable reference. `position` names the
    // syntactic position for the error message only.
    let mut resolve_conjunct =
        |conjunct: &smelt_parser::Expr, position: &str| -> Result<(), String> {
            for cref in collect_column_refs(conjunct) {
                let Some(leaf) = resolve_reference_leaf(tree, cref.qualifier(), cref.name()) else {
                    return Err(format!(
                        "{position} column reference '{}' does not resolve to a base-relation \
                     column the composition walk can chase",
                        cref.name()
                    ));
                };
                let bare = leaf
                    .relation
                    .strip_prefix("sources.")
                    .unwrap_or(leaf.relation.as_str())
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
    // explicitly names, but this pass never recurses into a nested
    // SELECT's own FROM/aliases — it fails closed instead of silently
    // treating the predicate as if it read nothing.
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
