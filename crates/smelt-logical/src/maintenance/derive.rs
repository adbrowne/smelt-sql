//! Pure derivation of a [`MaintenancePlan`] from analysis facts — v0.
//!
//! Consumes the derivations that exist (`analysis::source_bounds` for reach,
//! `analysis::discriminants` for combiner algebra, `analysis::model_diff` for
//! the additive-only column-add proof) and takes as *inputs* the two
//! classifiers that do not exist yet (column groups, skeleton roles) — see
//! the module doc in [`super`].

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use smelt_parser::syntax_kind::SyntaxNode;
use smelt_parser::{ColumnRef, Expr};
use smelt_types::SqlFunction;

use super::{
    ColumnGroup, Corner, FingerprintProjection, Grain, MaintenancePlan, MutationProfile,
    OutputSpec, PartitionLocal, PlanCell, Refusal, RowIdentity, RowIdentityVerdict, ScanClamp,
    SourceFacts, Technique, Trigger,
};
use crate::analysis::definition_change::{
    classify_definition_change, DefinitionChangeClass, DefinitionChangeCtx,
};
use crate::analysis::faithful_fold::{faithful_fold, ConditionVerdict, FaithfulFold};
use crate::analysis::fingerprint::fingerprint_projection;
use crate::analysis::footprint::{reflect_footprint, FootprintResult};
use crate::analysis::input_delta::{
    input_delta_discovery, MutationProfile as DeltaMutationProfile, SourceShape,
};
use crate::analysis::join_shape::JoinContext;
use crate::analysis::locality_projection::{locality_verdict, LocalityVerdict};
use crate::analysis::model_diff::ColumnDef;
use crate::analysis::output_delta::OutputDelta;
use crate::analysis::source_bounds::{
    derive_cross_axis_links, derive_model_bounds, resolve_table_ref_source_name, BoundContext,
    BoundResult, CrossAxisLink, Seconds,
};
use crate::analysis::walk::model_property_vector;
use crate::analysis::{item_alias, item_expr, select_stmt_items, SelectItemKind};
use crate::maintenance::repair;
use crate::maintenance::skeleton::skeleton_roles;

/// Derive the region row identity (P2, `model_properties.md` §"Region row
/// identity") for a model: the declared `unique_key` off the output's own
/// `Grain::Key` when present, else the proven grain key the composition walk
/// establishes over `sql` (`analysis::walk::PropertyVector::grain`), else the
/// identity-free `WholeRow` fallback.
///
/// Fail-closed: a proven key is only trusted when the walk also proves no
/// input join fans the output out (`PropertyVector::has_fan_out_join`) — a
/// key that does not cover every output row is never used, not even as a
/// partial key. `declared_unique_key` and a differing proven key may both be
/// present at once; declared wins the precedence, but the disagreement is
/// carried in [`RowIdentityVerdict::proven_mismatch`] rather than silently
/// dropped.
pub fn row_identity(declared_unique_key: &[String], sql: &str) -> RowIdentityVerdict {
    row_identity_with_context(declared_unique_key, sql, &JoinContext::new())
}

/// [`row_identity`], but folding an explicit [`JoinContext`] into the walk's
/// fan-out check instead of an always-empty one. Used by
/// [`append_model_edge_cells`] (T3, `docs/plans/20260715-composed-axes-
/// conditional-maintenance.md` Phase E3) so a model-edge cell's row-identity
/// proof can trust a proven grain key across an enrichment join whose
/// partner's row-uniqueness is already an established fact — the SAME
/// per-edge declared `unique_key` fact [`model_edge_enrichment_closure`]'s
/// P1 proof already folds into its own `ctx` for the identical join, never a
/// second, independent guess at the partner's uniqueness. Every other caller
/// (via [`row_identity`]) is unaffected — an empty `ctx` reproduces exactly
/// the pre-existing fail-closed behaviour (any join is untrusted absent an
/// external fact).
pub fn row_identity_with_context(
    declared_unique_key: &[String],
    sql: &str,
    ctx: &JoinContext,
) -> RowIdentityVerdict {
    let proven_key = model_property_vector(sql, ctx).and_then(|vector| {
        if vector.has_fan_out_join {
            None
        } else {
            vector.grain.keys.into_iter().next()
        }
    });

    if !declared_unique_key.is_empty() {
        let declared = declared_unique_key.to_vec();
        let proven_mismatch = proven_key.filter(|proven| !same_key_set(proven, &declared));
        return RowIdentityVerdict {
            identity: RowIdentity::Key(declared),
            proven_mismatch,
        };
    }

    match proven_key {
        Some(key) => RowIdentityVerdict {
            identity: RowIdentity::Key(key),
            proven_mismatch: None,
        },
        None => RowIdentityVerdict {
            identity: RowIdentity::WholeRow,
            proven_mismatch: None,
        },
    }
}

/// Derive a model's full [`Trigger`] set from pure facts — the single
/// derivation `docs/specs/incremental_models.md` §"Per-cell admission"'s
/// "Which changed inputs get a mutation cell" paragraph describes. One
/// `Trigger::NewData` per distinct source (declaration order,
/// first-occurrence deduplicated — a source repeated in `sources`, e.g. once
/// per alias, contributes exactly one creation trigger), one
/// `Trigger::UpstreamMutation` per source the rule below admits, always
/// `Trigger::Backfill`, and `Trigger::ColumnAdded` iff `added_columns` is
/// non-empty.
///
/// A source gets an `UpstreamMutation` cell iff it **explicitly** declares
/// `mutation_profile: mutable_snapshot` (named in `explicitly_mutable` — the
/// fail-closed admission default alone never synthesises one; an undeclared
/// source is not silently treated as mutable) **or** it is `AppendOnly` and
/// named in some [`ColumnGroup::mutation_sensitivity`] (a late append into an
/// already-written region changes stored values, so that region is
/// maintained, not left stale).
///
/// The source's clock (`SourceFacts::partition_col`) is deliberately not
/// part of this rule: whether the resulting cell's scan can be clamped to
/// the output partition axis is a downstream *admission* question
/// (`project_source_link`'s locality proof), not a derivation-time gate. A
/// clocked mutable source whose scan the locality proof cannot clamp
/// surfaces the ordinary `MaintenanceScanUnbounded` refusal — escapable by
/// `allow_full_scan` / `scan_bounds.on_violation: warn` — the same loud path
/// an unclocked one already takes, never a silently-dropped cell.
pub fn derive_triggers(
    sources: &[SourceFacts],
    column_groups: &[ColumnGroup],
    explicitly_mutable: &HashSet<String>,
    added_columns: &[String],
) -> Vec<Trigger> {
    let mut triggers = Vec::new();
    let mut seen = BTreeSet::new();
    for s in sources {
        if !seen.insert(s.name.clone()) {
            continue;
        }
        triggers.push(Trigger::NewData {
            source: s.name.clone(),
        });
        let gets_mutation_cell = match s.mutation {
            MutationProfile::MutableSnapshot => explicitly_mutable.contains(&s.name),
            // A change feed can only arise from an explicit declaration —
            // there is no fail-closed default it could be silently
            // conflated with — so the declaration alone suffices, unlike
            // `MutableSnapshot`'s `explicitly_mutable` gate.
            MutationProfile::ChangeFeed => true,
            MutationProfile::AppendOnly => column_groups
                .iter()
                .any(|g| g.mutation_sensitivity.contains(&s.name)),
        };
        if gets_mutation_cell {
            triggers.push(Trigger::UpstreamMutation {
                source: s.name.clone(),
            });
        }
    }
    triggers.push(Trigger::Backfill);
    if !added_columns.is_empty() {
        triggers.push(Trigger::ColumnAdded {
            columns: added_columns.to_vec(),
        });
    }
    triggers
}

/// Order-independent, case-insensitive key-set equality — the same
/// convention `Grain::has_subset_key` and the key-temporal-locality route's
/// `unique_key` comparison use.
fn same_key_set(a: &[String], b: &[String]) -> bool {
    let a: BTreeSet<String> = a.iter().map(|c| c.to_ascii_lowercase()).collect();
    let b: BTreeSet<String> = b.iter().map(|c| c.to_ascii_lowercase()).collect();
    a == b
}

/// The [`SourceShape`] [`input_delta_discovery`] reads for `facts`: a
/// clocked source's own partition column stands in for
/// `SourceShape::has_clock` (`SourceFacts::partition_col`'s doc comment: "the
/// source's partition column, when it is clocked"), and the plan-layer
/// [`MutationProfile`] maps onto the analysis-layer one 1:1.
fn source_shape(facts: &SourceFacts) -> SourceShape {
    SourceShape {
        has_clock: facts.partition_col.is_some(),
        mutation_profile: Some(match facts.mutation {
            MutationProfile::AppendOnly => DeltaMutationProfile::AppendOnly,
            MutationProfile::MutableSnapshot => DeltaMutationProfile::Mutable,
            MutationProfile::ChangeFeed => DeltaMutationProfile::ChangeFeed,
        }),
    }
}

/// Caller-supplied fold admission input for a keyed-grain model: which
/// columns fold additively, each under its **own** combiner (a mixed fold —
/// e.g. `COUNT`→`SUM`, `MIN`→`MIN`, `MAX`→`MAX` composed over the same
/// key — is the common shape, not a single shared combiner across every
/// column). Checked against the combiner-algebra classifier per column —
/// never trusted bare.
#[derive(Debug, Clone)]
pub struct FoldSpec {
    pub add_columns: Vec<(String, SqlFunction)>,
}

/// Whether `source` contributes to `sql`'s cumulative fold — i.e. appears as
/// an argument to one of the fold's own aggregate expressions
/// (`model_properties.md` §"Faithful-fold conditions";
/// `docs/specs/incremental_shapes.md` §"The key grain (`grain: key`)" binds
/// the `NewData` append-only obligation to exactly the sources this
/// classifier answers `true` for, not every source the model references).
/// [`FoldSpec`] itself carries no source attribution (`add_columns` is
/// `(alias, combiner)` only), so this re-parses `sql`'s own outermost
/// `SELECT` to see which FROM source backs each aggregate's arguments.
///
/// **Leaf classifier** — runs over the model's own already-bounded fold
/// body (the outermost `SELECT`'s own aggregate expressions and FROM-alias
/// map); feeds admission; never composes across nodes. Matches
/// `maintenance::grouping`'s per-column mutation-sensitivity classifier's
/// own v0 scope restriction: single top-level `SELECT` scope only, no
/// CTE/derived-table chase (`docs/specs/architecture.md` §"Property
/// composition walk rule" — admissible as a leaf classifier invoked over one
/// already-bounded node's own text, not a competing whole-model scan).
///
/// **Conservative by construction — false negatives are forbidden.** Every
/// case this classifier cannot resolve cleanly — an unqualified column
/// reference ambiguous among more than one joined FROM source, a qualifier
/// that does not resolve to a named FROM alias, a derived-table/subquery
/// FROM item, or a CTE/set-operation composing the fold body through more
/// than this one scope — answers `true` ("contributes"), never `false`. A
/// false positive here only costs permissiveness (the caller's separate
/// `UpstreamMutation`-coverage check still has to hold before anything is
/// admitted); a false negative would silently let an un-retractable folded
/// contribution through, which is exactly the admission hole this predicate
/// exists to close.
pub fn source_contributes_to_fold(sql: &str, source: &str) -> bool {
    let stripped = crate::types::Frontmatter::strip(sql);
    let parse = smelt_parser::parse(stripped);
    let Some(file) = smelt_parser::File::cast(parse.syntax()) else {
        return true;
    };
    let Some(select) = file.select_stmt() else {
        return true;
    };
    let Some(items) = select_stmt_items(&select) else {
        return true;
    };

    // A CTE or set operation composes the fold body through more than one
    // scope — outside this leaf classifier's single-scope resolution
    // (matches `maintenance::grouping`'s own v0 restriction). Cannot prove
    // `source` absent from a scope this classifier does not chase into.
    if select.with_clause().is_some() || select.has_set_operation() {
        return true;
    }

    let Some(from_clause) = select.from_clause() else {
        return true;
    };

    let mut aliases: BTreeMap<String, String> = BTreeMap::new();
    for table_ref in from_clause
        .table_refs()
        .chain(from_clause.joins().filter_map(|j| j.table_ref()))
    {
        if table_ref.subquery().is_some() {
            // A derived table this classifier does not chase into — cannot
            // prove it doesn't itself surface `source`.
            return true;
        }
        let Some(resolved) = resolve_table_ref_source_name(&table_ref) else {
            return true;
        };
        let bare = resolved
            .strip_prefix("sources.")
            .unwrap_or(resolved.as_str())
            .to_string();
        let key = table_ref
            .alias()
            .unwrap_or_else(|| bare.clone())
            .to_ascii_lowercase();
        aliases.insert(key, bare);
    }

    let source_lower = source.to_ascii_lowercase();
    // `source` does not even appear as a FROM item under any name/alias
    // this classifier resolved: the fold body cannot read it at all, under
    // any qualifier, from this scope.
    if !aliases
        .values()
        .any(|v| v.eq_ignore_ascii_case(&source_lower))
    {
        return false;
    }

    for item in &items {
        let is_aggregate = matches!(
            item,
            SelectItemKind::OtherAggregate { .. } | SelectItemKind::CountDistinct { .. }
        );
        if !is_aggregate {
            continue;
        }
        for cref in collect_fold_column_refs(item_expr(item)) {
            let resolved = match cref.qualifier() {
                Some(q) => aliases.get(&q.to_ascii_lowercase()).cloned(),
                // Unqualified inside an aggregate argument: resolvable only
                // when exactly one source is joined in this scope — with
                // more than one, it is ambiguous and cannot be proven not
                // to be `source`.
                None if aliases.len() == 1 => aliases.values().next().cloned(),
                None => None,
            };
            match resolved {
                Some(name) if name.eq_ignore_ascii_case(&source_lower) => return true,
                Some(_) => continue,
                // A qualifier this FROM-alias scan didn't resolve — cannot
                // prove it is not an alias for `source`.
                None => return true,
            }
        }
    }
    false
}

/// Recursively collect every simple (possibly qualified) column reference
/// inside `expr` — a leaf classifier over one already-parsed aggregate
/// argument's own syntax tree, mirroring `maintenance::grouping`'s private
/// helper of the same shape (kept local; that one is not `pub`).
fn collect_fold_column_refs(expr: &Expr) -> Vec<ColumnRef> {
    let mut out = Vec::new();
    collect_fold_column_refs_rec(expr.syntax(), &mut out);
    out
}

fn collect_fold_column_refs_rec(node: &SyntaxNode, out: &mut Vec<ColumnRef>) {
    if node.kind() == smelt_parser::SyntaxKind::EXPRESSION {
        if let Some(e) = Expr::cast(node.clone()) {
            if let Some(cref) = ColumnRef::from_expr(&e) {
                out.push(cref);
                return;
            }
        }
    }
    for child in node.children() {
        collect_fold_column_refs_rec(&child, out);
    }
}

/// One upstream **maintained-model** edge (`incremental_models.md` §"Upstream
/// model edges"): a downstream maintained model's ref to another maintained
/// model in the same project. Built by the caller from the upstream's own
/// already-validated metadata — the derivation never re-resolves the ref.
#[derive(Debug, Clone)]
pub struct ModelEdge {
    /// The upstream model's address as it appears in the downstream ref, with
    /// the leading `smelt.` stripped (e.g. `silver.events_parsed`). Used as
    /// the edge's `Trigger::NewData` source name and the clamp's source.
    pub name: String,
    /// The upstream's own validated `timeseries.partition_column`, when it
    /// declares (or infers) one. `None` ⇒ the clock is not derivable ⇒ a
    /// recorded [`Refusal::ReachNotDerivable`] naming the edge, never a
    /// silent drop.
    pub clock_col: Option<String>,
    /// The upstream's own declared top-level `unique_key:`
    /// (`docs/specs/models.md` §"The Relation Contract"), when any. Empty
    /// when the upstream declares none — this edge then contributes no
    /// [`crate::analysis::join_shape::JoinContext`] fact and a join against
    /// it cannot be proven one-to-one, so P1 skeleton-source closure
    /// (T3, `docs/plans/20260715-composed-axes-conditional-maintenance.md`
    /// Phase E3) stays `Open` for it rather than optimistically assuming
    /// uniqueness.
    pub unique_key: Vec<String>,
    /// Sibling spellings of `clock_col` within the upstream's own SQL —
    /// other select-item aliases whose defining expression is textually
    /// identical to `clock_col`'s own
    /// ([`crate::analysis::source_bounds::defining_expr_siblings`]), hence
    /// provably equal to it. Threaded into the cross-axis-link derivation
    /// ([`crate::analysis::source_bounds::derive_cross_axis_links`]) so a
    /// downstream predicate anchored on a same-value sibling column (kept,
    /// e.g., for a pre-existing consumer's column-name compatibility) links
    /// exactly as an anchor on `clock_col` itself would. Empty when
    /// `clock_col` is `None` or has no such sibling — never a guess.
    pub clock_col_aliases: Vec<String>,
    /// The upstream's own derived output-delta shape
    /// (`crate::analysis::output_delta::OutputDelta`,
    /// `incremental_models.md` §"The graph layer" → "Typed edges"), scalar
    /// per edge — the caller's own meet across whatever per-column-group
    /// verdicts it derived for the upstream (this admission gate does not
    /// need the fine per-column-group resolution `type_edge` does; a coarse
    /// scalar only ever widens which edges take the key-addressed route,
    /// never narrows one incorrectly). `None` when the caller has not
    /// derived one — today's default, unaffected clock-only admission.
    pub output_shape: Option<crate::analysis::output_delta::OutputDelta>,
}

/// Append the creation-trigger cells (and refusals) for `model_edges` to an
/// already-derived `plan` (`incremental_models.md` §"Upstream model edges").
///
/// Kept separate from [`derive_maintenance_plan`] so every existing
/// source-only caller is unaffected: the assembler calls both and merges the
/// results into one plan (still one derivation, purely data-in/data-out).
///
/// A clocked upstream contributes a `{*}` creation cell whose scan clamp is
/// anchored to the downstream's output partition axis via the same
/// [`project_source_link`] locality proof sources use; an upstream with no derivable clock is a
/// [`Refusal::ReachNotDerivable`] naming the edge. Model edges only
/// contribute to a **partition-addressed** downstream (`output_partition_col`
/// is `Some`); a key-addressed downstream's model-edge creation is a keyed
/// fold, out of scope here.
///
/// `declared_unique_key` is the downstream's own declared `unique_key:`
/// (`docs/specs/models.md` §"Refresh axis"), threaded into the same
/// [`row_identity`] derivation `derive_maintenance_plan` uses for this
/// model's other cells, so a model-edge creation cell carries the identical
/// row-identity verdict as every other cell of the same output.
pub fn append_model_edge_cells(
    plan: &mut MaintenancePlan,
    sql: &str,
    output_partition_col: Option<&str>,
    model_edges: &[ModelEdge],
    declared_unique_key: &[String],
) {
    if model_edges.is_empty() {
        return;
    }
    // The `JoinContext` built from every joined edge's own declared
    // `unique_key` (see `model_edges_join_context`'s doc comment) — shared
    // by the row-identity proof below AND `model_edge_enrichment_closure`'s
    // P1 proof, so both properties of this SAME model-edge cell see the
    // SAME declared facts rather than the row-identity proof working from a
    // second, independent (and always-empty) context.
    let join_ctx = model_edges_join_context(sql, model_edges);
    let identity = row_identity_with_context(declared_unique_key, sql, &join_ctx);
    // P1 skeleton-source closure — a property of the model's own query
    // shape (`model_edge_enrichment_closure`'s doc comment below), so it is
    // shared by both the key-addressed loop right below and the
    // partition-addressed loop further down, computed once.
    let enrichment_closure = model_edge_enrichment_closure(sql, model_edges, &join_ctx);

    // Key-addressed edges (`incremental_models.md` §"Upstream model edges"):
    // an upstream whose own derived output-delta shape is `KeyedUpsert`
    // contributes a key-addressed `PerGroupRecompute` cell whenever the
    // clock-based route below has nothing to admit anyway — a clockless
    // upstream, or a keyed-grain downstream with no partition axis to clamp
    // against (checked BEFORE the `output_partition_col`/`clock_col` gates
    // below). A clocked upstream feeding a partition-addressed downstream
    // keeps today's `DeleteInsert` route unchanged — both routes are
    // admissible for that shape, and narrowing (never widening) which edges
    // move to the new route keeps every existing clock-based fixture's
    // technique stable. A `KeysNotDiscoverable`/`SliceUnbounded` refusal
    // from `admit_key_addressed_recompute` (the downstream SQL does not
    // carry the upstream's key columns, or has no derivable grain of its
    // own) is recorded by name — never a silent fallback to a whole-table
    // cell.
    for edge in model_edges {
        let Some(OutputDelta::KeyedUpsert { keys }) = &edge.output_shape else {
            continue;
        };
        let clock_route_applies = edge.clock_col.is_some() && output_partition_col.is_some();
        if clock_route_applies {
            continue;
        }
        match repair::admit_key_addressed_recompute(
            sql,
            declared_unique_key,
            &edge.name,
            keys,
            &join_ctx,
        ) {
            Ok(key_scope) => {
                plan.cells.push(PlanCell {
                    group: "{*}".to_string(),
                    trigger: Trigger::NewData {
                        source: edge.name.clone(),
                    },
                    corner: Corner::ColumnMerge,
                    technique: Technique::PerGroupRecompute,
                    // Honest: this cell claims no partition-interval scan —
                    // its bounded read is the key set on `key_scope` instead
                    // (`PlanCell::key_scope`'s doc comment).
                    partition_local: PartitionLocal::No {
                        source: edge.name.clone(),
                        why: format!(
                            "upstream maintained model '{}' is key-addressed (a KeyedUpsert \
                             output-delta shape) — its fold restricts to the affected key set, \
                             not a partition interval",
                            edge.name
                        ),
                    },
                    scans: vec![],
                    ledger_catch_up: false,
                    row_identity: identity.clone(),
                    skeleton_source_closure: enrichment_closure.clone(),
                    // P4 is defined over external sources, not upstream
                    // maintained models — matches every other model-edge
                    // cell's own empty verdict.
                    fingerprint_projections: BTreeMap::new(),
                    key_scope: Some(key_scope),
                });
            }
            Err(refusal) => {
                let (source, why) = match refusal {
                    repair::RepairRefusal::KeysNotDiscoverable { source, why } => (source, why),
                    repair::RepairRefusal::SliceUnbounded { source, why } => (source, why),
                };
                plan.refusals
                    .push(Refusal::RepairKeysNotDiscoverable { source, why });
            }
        }
    }

    // A key-addressed downstream has no partition axis to clamp a
    // *partition*-addressed creation cell to — the remaining (non-
    // `KeyedUpsert`) edges have nothing left to admit here.
    let Some(output_partition_col) = output_partition_col else {
        return;
    };

    // Derive per-edge bounds over the downstream SQL, keyed by each clocked
    // edge's clock column — the same Form A/B extraction sources use.
    let mut ctx = BoundContext::new();
    for edge in model_edges {
        if let Some(clock) = &edge.clock_col {
            ctx.add_source(&edge.name, clock);
            ctx.add_source_partition_col_aliases(&edge.name, edge.clock_col_aliases.clone());
        }
    }
    let bounds = derive_model_bounds(sql, &ctx);
    // The write-scope dual of `bounds` over the same edge context
    // (`model_properties.md` §"Footprint reflection / bounded write
    // footprint") — consulted by the locality proof before constructing each
    // edge's clamp.
    let footprints = reflect_footprint(sql, &ctx, Some(output_partition_col));
    // The cross-axis predicate evidence over the same edge context
    // (`model_properties.md` §"Partition-locality projection") — the third
    // input the locality proof composes.
    let links = derive_cross_axis_links(sql, &ctx, output_partition_col);

    // `enrichment_closure` (P1 skeleton-source closure) was already derived
    // above, before the key-addressed loop — shared, not re-derived, by this
    // (partition-addressed) loop's cells too.

    for edge in model_edges {
        if matches!(edge.output_shape, Some(OutputDelta::KeyedUpsert { .. }))
            && edge.clock_col.is_none()
        {
            // Already admitted or refused by the key-addressed loop above
            // (a clockless `KeyedUpsert` edge — `output_partition_col` is
            // `Some` here, by this point past the early return, so the key
            // loop's `clock_route_applies` for this edge was `false`).
            continue;
        }
        let Some(clock) = &edge.clock_col else {
            plan.refusals.push(Refusal::ReachNotDerivable {
                edge: edge.name.clone(),
                why: format!(
                    "upstream maintained model '{}' declares no timeseries clock and none is \
                     inferable — its creation-trigger edge cannot be clamped to the output \
                     partition axis",
                    edge.name
                ),
            });
            continue;
        };
        let facts = SourceFacts {
            name: edge.name.clone(),
            mutation: MutationProfile::AppendOnly,
            partition_col: Some(clock.clone()),
            unique_key: vec![],
            allow_full_scan: false,
        };
        // A creation-trigger recompute is unconditionally valid (like the
        // `NewData`/`Backfill` region recompute), so an unlinked edge records
        // a non-local verdict but is never refused under the K8 guardrail —
        // only the *underivable-clock* case above refuses.
        let loc = LocalityInputs {
            bounds: &bounds,
            footprints: &footprints,
            links: &links,
        };
        let (partition_local, scans) =
            match project_source_link(Some(output_partition_col), None, &loc, &facts) {
                SourceLink::Clamp(clamp) => (PartitionLocal::Yes, vec![clamp]),
                SourceLink::Unlinked { why } => (
                    PartitionLocal::No {
                        source: edge.name.clone(),
                        why,
                    },
                    vec![],
                ),
                // Unreachable: `facts.partition_col` is `Some` by construction.
                SourceLink::Unclocked => (
                    PartitionLocal::No {
                        source: edge.name.clone(),
                        why: "model edge lost its clock column".to_string(),
                    },
                    vec![],
                ),
            };
        plan.cells.push(PlanCell {
            group: "{*}".to_string(),
            trigger: Trigger::NewData {
                source: edge.name.clone(),
            },
            corner: Corner::RecomputeRegion,
            technique: Technique::DeleteInsert,
            partition_local,
            scans,
            ledger_catch_up: false,
            row_identity: identity.clone(),
            skeleton_source_closure: enrichment_closure.clone(),
            // P4 is defined over external sources, not upstream maintained
            // models — a model-edge cell carries no fingerprint-projection
            // verdicts (`PlanCell::fingerprint_projections`'s documented
            // empty case).
            fingerprint_projections: BTreeMap::new(),
            key_scope: None,
        });
    }
}

/// Build the [`JoinContext`] `analysis::join_shape::fan_out`'s one-to-one
/// conjunct needs from every one of `model_edges` that is actually joined in
/// `sql` (resolved via `analysis::skeleton_closure::enrichment_join_alias`,
/// never guessed), keyed by each joined edge's own declared `unique_key`.
/// Shared by [`model_edge_enrichment_closure`]'s P1 proof and
/// [`append_model_edge_cells`]'s P2 row-identity proof — both properties of
/// the SAME model-edge cell see the SAME declared-unique-key facts. An edge
/// whose `unique_key` is undeclared, or whose alias this resolves to `None`
/// for (it is not actually joined in this scope, e.g. it is the
/// `FROM`-clause driving table), contributes no key fact — a join against it
/// fails closed exactly as it would with no `JoinContext` entry at all.
fn model_edges_join_context(sql: &str, model_edges: &[ModelEdge]) -> JoinContext {
    use crate::analysis::skeleton_closure::enrichment_join_alias;

    let mut ctx = JoinContext::new();
    for edge in model_edges {
        let Some(alias) = enrichment_join_alias(sql, &edge.name) else {
            continue;
        };
        if !edge.unique_key.is_empty() {
            let cols: Vec<&str> = edge.unique_key.iter().map(String::as_str).collect();
            ctx = ctx.with_composite_unique_key(&alias, &cols);
        }
    }
    ctx
}

/// Derive the shared P1 skeleton-source-closure verdict for a model's
/// upstream model edges (see [`append_model_edge_cells`]'s call site doc
/// comment for why this is one derivation shared across every edge's cell,
/// not a per-edge one). `join_ctx` is [`model_edges_join_context`]'s output
/// — the same one the caller also feeds to the row-identity proof, never a
/// second, independently-built context.
fn model_edge_enrichment_closure(
    sql: &str,
    model_edges: &[ModelEdge],
    join_ctx: &JoinContext,
) -> Option<crate::analysis::skeleton_closure::SkeletonSourceClosure> {
    use crate::analysis::skeleton_closure::{enrichment_join_alias, skeleton_source_closure};

    let joined_edges: Vec<&ModelEdge> = model_edges
        .iter()
        .filter(|edge| enrichment_join_alias(sql, &edge.name).is_some())
        .collect();
    if joined_edges.is_empty() {
        return None;
    }
    let mut verdict = crate::analysis::skeleton_closure::SkeletonSourceClosure::Closed {
        row_preservation: crate::analysis::skeleton_closure::RowPreservation::JoinShape,
    };
    for edge in joined_edges {
        let v = skeleton_source_closure(sql, &edge.name, None, join_ctx);
        if !v.is_closed() {
            verdict = v;
            break;
        }
    }
    Some(verdict)
}

/// External-source `referential_integrity:` world-facts (`docs/specs/
/// sources.md` §"Referential integrity"), keyed by source name (matching
/// [`SourceFacts::name`]), consumed by [`mutation_enrichment_closure`] for
/// P1's row-preservation conjunct (4) on an `UpstreamMutation` cell's own
/// enrichment join (T3 over external sources, `docs/plans/
/// 20260715-composed-axes-conditional-maintenance.md` Phase F5). A source
/// with no entry contributes no row-preservation fact — its enrichment
/// join's closure proof is never attempted (`None`, not a disproven
/// `Open`), matching [`derive_maintenance_plan`]'s own always-empty-map
/// call, which is byte-identical to its pre-F5 behaviour.
pub type SourceReferentialIntegrity = BTreeMap<String, Vec<String>>;

/// Build the [`JoinContext`] [`mutation_enrichment_closure`]'s one-to-one
/// conjunct (3) needs from every one of `sources` that is actually joined
/// in `sql` (resolved via `analysis::skeleton_closure::enrichment_join_
/// alias`, never guessed), keyed by each joined source's own declared
/// `unique_key` (`SourceFacts::unique_key`). Mirrors [`model_edges_join_
/// context`] exactly, generalized from upstream maintained-model edges to
/// external sources — a source whose `unique_key` is undeclared, or whose
/// alias this resolves to `None` for (not actually joined in this scope,
/// e.g. it is the `FROM`-clause driving table), contributes no key fact,
/// same fail-closed default as the model-edge case.
fn source_facts_join_context(sql: &str, sources: &[SourceFacts]) -> JoinContext {
    use crate::analysis::skeleton_closure::enrichment_join_alias;

    let mut ctx = JoinContext::new();
    for facts in sources {
        let Some(alias) = enrichment_join_alias(sql, &facts.name) else {
            continue;
        };
        if !facts.unique_key.is_empty() {
            let cols: Vec<&str> = facts.unique_key.iter().map(String::as_str).collect();
            ctx = ctx.with_composite_unique_key(&alias, &cols);
        }
    }
    ctx
}

/// Derive the P1 skeleton-source-closure verdict for an `UpstreamMutation`
/// cell's own enrichment join against `source` — the external-source
/// analogue of [`model_edge_enrichment_closure`] (T3 over external sources,
/// `docs/plans/20260715-composed-axes-conditional-maintenance.md` Phase
/// F5): the SAME [`skeleton_source_closure`] proof, fed the source's
/// declared `referential_integrity` world-fact instead of a model edge's
/// always-`None` one (an external source has no upstream `unique_key`
/// analogue to license row preservation on its own — only its own declared
/// `referential_integrity` can).
///
/// `None` when `source_referential_integrity` carries no entry for
/// `source` — the caller opted out of the closure proof entirely for this
/// source, exactly matching every `UpstreamMutation` cell's behaviour
/// before this map existed (`derive_maintenance_plan`'s own call always
/// passes an empty map). When an entry *is* present, a declared
/// `referential_integrity` alone does not guarantee `Closed`:
/// [`skeleton_source_closure`] still independently checks every conjunct
/// (including one-to-one join contribution via [`source_facts_join_
/// context`]'s declared-`unique_key` facts, and the v1 aggregation-scope
/// restriction), so a caller that declares `referential_integrity` without
/// a matching `unique_key`, or over a fan-out join, still correctly sees
/// `Open`.
fn mutation_enrichment_closure(
    sql: &str,
    source: &str,
    sources: &[SourceFacts],
    source_referential_integrity: &SourceReferentialIntegrity,
) -> Option<crate::analysis::skeleton_closure::SkeletonSourceClosure> {
    use crate::analysis::skeleton_closure::skeleton_source_closure;

    let ri = source_referential_integrity.get(source)?;
    let join_ctx = source_facts_join_context(sql, sources);
    Some(skeleton_source_closure(sql, source, Some(ri), &join_ctx))
}

/// Everything the v0 derivation reads. `column_groups` and
/// `output.skeleton_columns` are hand-supplied (the deferred classifiers);
/// the rest is derived from `sql` and the source declarations.
#[derive(Debug)]
pub struct ModelInputs<'a> {
    /// Expanded model SQL (functions inlined), used for bound derivation.
    pub sql: &'a str,
    pub output: OutputSpec,
    pub sources: Vec<SourceFacts>,
    pub column_groups: Vec<ColumnGroup>,
    /// Present for keyed-grain models whose new-data cell should fold.
    pub fold: Option<FoldSpec>,
    /// The model's existing (pre-`ColumnAdded`) output columns — the
    /// [`crate::analysis::definition_change::classify_definition_change`]
    /// proof's `old_columns` for a `ColumnAdded` trigger. Empty means "no
    /// old schema known", which fails closed exactly as the retired
    /// `column_add_proof: None` did (`docs/plans/
    /// 20260808-derived-maintenance-proofs.md` Phase 4).
    pub old_columns: Vec<ColumnDef>,
    /// The model's previously-deployed source SQL text (world-fact, from
    /// the deployed-schema snapshot's `model_sql`, supplied by the caller —
    /// this module does no I/O of its own). `None` means "no known deployed
    /// SQL", which derives no [`Refusal::SkeletonClauseChanged`] at all —
    /// fail-closed, the same posture `old_columns`/`deployed_column_names`
    /// take (`docs/specs/definition_deltas.md` §"Detection").
    pub old_sql: Option<&'a str>,
    /// The declared `timeseries.partition_column` of a `Grain::Key` model,
    /// `None` for a `Grain::Partition` output or a keyed output with no
    /// declared `timeseries:` block. Poses the footprint question
    /// (`model_properties.md` §"Footprint reflection / bounded write
    /// footprint") for a keyed-grain output against this axis; a bare
    /// keyed output (`None` here) gets no footprint claim at all.
    pub keyed_time_axis: Option<&'a str>,
}

impl ModelInputs<'_> {
    fn source(&self, name: &str) -> Option<&SourceFacts> {
        self.sources.iter().find(|s| s.name == name)
    }

    fn bound_context(&self) -> BoundContext {
        let mut ctx = BoundContext::new();
        for s in &self.sources {
            if let Some(p) = &s.partition_col {
                ctx.add_source(&s.name, p);
            }
        }
        ctx
    }

    fn output_partition_col(&self) -> Option<&str> {
        match &self.output.grain {
            Grain::Partition { partition_col } => Some(partition_col),
            Grain::Key { .. } => None,
        }
    }

    /// The declared identity off the output's own grain (P2, `row_identity`):
    /// `Grain::Key`'s `unique_key`, or nothing for `Grain::Partition` — a
    /// partition-grain output declares no row-level identity through
    /// `Grain` itself.
    fn declared_unique_key(&self) -> &[String] {
        match &self.output.grain {
            Grain::Key { unique_key } => unique_key,
            Grain::Partition { .. } => &[],
        }
    }
}

/// How one read source relates to the output's partition axis for a
/// region-anchored maintenance op.
pub enum SourceLink {
    /// Bounded: the derived scan clamp, anchored to the output region.
    Clamp(ScanClamp),
    /// Clocked but with no derivable link to the output partition axis (or
    /// an unbounded one) — the op cannot be partition-pruned.
    Unlinked { why: String },
    /// Not clocked at all: a lookup read in full.
    Unclocked,
}

/// The three per-source derivations the partition-locality projection
/// composes (`model_properties.md` §"Partition-locality projection") —
/// read bounds ([`derive_model_bounds`]), reflected write footprints
/// ([`reflect_footprint`]), and cross-axis predicate evidence
/// ([`derive_cross_axis_links`]) — derived once per model (or per edge
/// context) and threaded together to every clamp-construction site.
pub struct LocalityInputs<'a> {
    pub bounds: &'a HashMap<String, BoundResult>,
    pub footprints: &'a HashMap<String, FootprintResult>,
    pub links: &'a HashMap<String, CrossAxisLink>,
}

/// Project one source's partition-locality verdict onto a [`SourceLink`] —
/// the adapter between the pure locality proof
/// ([`locality_verdict`] — `model_properties.md` §"Partition-locality
/// projection") and the clamp/refusal plumbing the derivation sites share.
///
/// For a **partition-addressed** output, the verdict is the proof's,
/// verbatim: read bound ([`derive_model_bounds`]) AND reflected write
/// footprint ([`reflect_footprint`]) must both be `Bounded`, and a
/// cross-axis source (its partition column is not the output's) is linked
/// only by the explicit-predicate evidence
/// ([`derive_cross_axis_links`]) — never by a nonzero scan margin. A
/// `Local` verdict over a `Bounded` read constructs the derived
/// [`ScanClamp`] exactly as before; every `NotLocal` reason becomes the
/// `Unlinked` why. Policy (creation cells never refuse; the K8
/// `ScanUnbounded` refusal honoring `allow_full_scan`) stays at each call
/// site — this adapter is policy-free.
///
/// For a **keyed-grain** output (`output_partition_col` is `None`) with a
/// declared `keyed_time_axis`, the footprint question IS posed against that
/// axis — `loc.footprints` must already be `reflect_footprint`'s verdicts
/// against it (the caller's responsibility, exactly as for a
/// partition-addressed output): a `Bounded` footprint constructs the clamp
/// carrying the derived write footprint; `Unbounded`/`NotDerivable` refuses
/// the link. With no declared `keyed_time_axis` at all, the pre-proof
/// linking rule is kept verbatim (a nonzero-margin `Bounded` read links, a
/// zero-margin one does not), but the resulting clamp carries no footprint
/// claim (`write_footprint: None`) — there is no axis to have posed it
/// against.
pub fn project_source_link(
    output_partition_col: Option<&str>,
    keyed_time_axis: Option<&str>,
    loc: &LocalityInputs<'_>,
    facts: &SourceFacts,
) -> SourceLink {
    let LocalityInputs {
        bounds,
        footprints,
        links,
    } = loc;
    let Some(col) = &facts.partition_col else {
        return SourceLink::Unclocked;
    };
    let Some(output_axis) = output_partition_col else {
        let read = bounds.get(&facts.name);
        return match keyed_time_axis {
            Some(_axis) => {
                let Some(BoundResult::Bounded { before, after, .. }) = read else {
                    return SourceLink::Unlinked {
                        why: "scan bound not derivable".to_string(),
                    };
                };
                match footprints
                    .get(&facts.name)
                    .unwrap_or(&FootprintResult::NotDerivable)
                {
                    FootprintResult::Bounded {
                        before: write_before,
                        after: write_after,
                        ..
                    } => SourceLink::Clamp(ScanClamp {
                        source: facts.name.clone(),
                        column: col.clone(),
                        before: *before,
                        after: *after,
                        write_footprint: Some((*write_before, *write_after)),
                    }),
                    FootprintResult::Unbounded => SourceLink::Unlinked {
                        why: "derived write footprint is unbounded against the declared time \
                              axis — a stored trajectory column"
                            .to_string(),
                    },
                    FootprintResult::NotDerivable => SourceLink::Unlinked {
                        why: "write footprint not derivable against the declared time axis"
                            .to_string(),
                    },
                }
            }
            None => match read {
                Some(BoundResult::Bounded { before, after, .. }) => {
                    if *before > Seconds::ZERO || *after > Seconds::ZERO {
                        SourceLink::Clamp(ScanClamp {
                            source: facts.name.clone(),
                            column: col.clone(),
                            before: *before,
                            after: *after,
                            write_footprint: None,
                        })
                    } else {
                        SourceLink::Unlinked {
                            why: format!(
                                "no predicate links '{col}' to the output partition axis — the \
                                 scan cannot be partition-pruned"
                            ),
                        }
                    }
                }
                Some(BoundResult::Unbounded) => SourceLink::Unlinked {
                    why: "derived scan is unbounded".to_string(),
                },
                Some(BoundResult::NotDerivable) | None => SourceLink::Unlinked {
                    why: "scan bound not derivable".to_string(),
                },
            },
        };
    };

    let read = bounds
        .get(&facts.name)
        .unwrap_or(&BoundResult::NotDerivable);
    // `bounds` and `footprints` are derived from the same SQL over the same
    // context, so their key sets agree; a source absent from both is refused
    // through the read leg (`NotDerivable`) before the footprint is read.
    let footprint = footprints
        .get(&facts.name)
        .unwrap_or(&FootprintResult::NotDerivable);
    let link = links
        .get(&facts.name)
        .copied()
        .unwrap_or(CrossAxisLink::Absent);

    match locality_verdict(read, footprint, Some(col), Some(output_axis), link) {
        LocalityVerdict::Local => match (read, footprint) {
            (
                BoundResult::Bounded { before, after, .. },
                FootprintResult::Bounded {
                    before: write_before,
                    after: write_after,
                    ..
                },
            ) => SourceLink::Clamp(ScanClamp {
                source: facts.name.clone(),
                column: col.clone(),
                before: *before,
                after: *after,
                write_footprint: Some((*write_before, *write_after)),
            }),
            // Unreachable: the proof only ever answers `Local` over a
            // `Bounded` read AND a `Bounded` footprint; kept fail-closed
            // rather than panicking.
            _ => SourceLink::Unlinked {
                why: "scan bound not derivable".to_string(),
            },
        },
        LocalityVerdict::NotLocal { reason } => SourceLink::Unlinked { why: reason },
    }
}

/// Derive the plan cells (and refusals) for `triggers` against `inputs`.
///
/// Every `UpstreamMutation` cell's `skeleton_source_closure` is `None` —
/// this entry point never attempts the P1 proof for an external source's
/// enrichment join (byte-identical to this function's pre-Phase-F5
/// behaviour). Use [`derive_maintenance_plan_with_referential_integrity`]
/// to opt an external source's declared `referential_integrity` world-fact
/// into the same proof [`append_model_edge_cells`] already runs for model
/// edges.
pub fn derive_maintenance_plan(inputs: &ModelInputs, triggers: &[Trigger]) -> MaintenancePlan {
    derive_maintenance_plan_impl(inputs, triggers, &SourceReferentialIntegrity::new())
}

/// [`derive_maintenance_plan`], additionally threading `source_referential_
/// integrity` world-facts (`docs/specs/sources.md` §"Referential
/// integrity") into every `UpstreamMutation` cell's P1 skeleton-source-
/// closure proof (T3 over external sources, `docs/plans/
/// 20260715-composed-axes-conditional-maintenance.md` Phase F5) — the
/// licence union with [`append_model_edge_cells`]'s already-landed model-
/// edge proof: the SAME [`crate::analysis::skeleton_closure::
/// skeleton_source_closure`] function, the SAME [`super::choice::
/// resolve_recompute_restriction`] gate downstream, no new mechanism.
///
/// A source absent from `source_referential_integrity` behaves exactly as
/// under [`derive_maintenance_plan`] (`skeleton_source_closure: None`) —
/// this function only *adds* closure attempts for the sources the caller
/// names, never removes or alters anything else `derive_maintenance_plan`
/// would have derived.
pub fn derive_maintenance_plan_with_referential_integrity(
    inputs: &ModelInputs,
    triggers: &[Trigger],
    source_referential_integrity: &SourceReferentialIntegrity,
) -> MaintenancePlan {
    derive_maintenance_plan_impl(inputs, triggers, source_referential_integrity)
}

fn derive_maintenance_plan_impl(
    inputs: &ModelInputs,
    triggers: &[Trigger],
    source_referential_integrity: &SourceReferentialIntegrity,
) -> MaintenancePlan {
    let mut plan = MaintenancePlan::default();
    if let Some(reason) = skeleton_clause_changed(inputs) {
        plan.refusals
            .push(Refusal::SkeletonClauseChanged { reason });
    }
    let bounds = derive_model_bounds(inputs.sql, &inputs.bound_context());
    // The write-scope dual of `bounds` (`model_properties.md` §"Footprint
    // reflection / bounded write footprint"), derived once per model and
    // consulted at every clamp-construction site via `project_source_link`. A
    // key-grain output has no partition axis to spread a write across, so
    // the footprint question is not posed (empty map — the keyed residue
    // policy in `project_source_link` links instead).
    let footprints = match inputs.output_partition_col().or(inputs.keyed_time_axis) {
        Some(axis) => reflect_footprint(inputs.sql, &inputs.bound_context(), Some(axis)),
        None => HashMap::new(),
    };
    // The cross-axis predicate evidence (`model_properties.md`
    // §"Partition-locality projection") — like the footprint, posed only
    // against a partition-addressed output (a keyed output has no axis to
    // link a source's partition column to; `project_source_link` keeps the
    // documented keyed residue policy instead).
    let links = match inputs.output_partition_col() {
        Some(axis) => derive_cross_axis_links(inputs.sql, &inputs.bound_context(), axis),
        None => HashMap::new(),
    };
    let loc = LocalityInputs {
        bounds: &bounds,
        footprints: &footprints,
        links: &links,
    };
    let identity = row_identity(inputs.declared_unique_key(), inputs.sql);
    // The set of sources this model's own trigger list covers with an
    // `UpstreamMutation` cell — i.e. every source name for which `triggers`
    // (built by the caller, e.g. `smelt-db::queries::maintenance::
    // derive_model_maintenance_plan`, under the unclocked +
    // `explicitly_mutable` predicate ≈ that function's L397-404) already
    // contains a `Trigger::UpstreamMutation`. Read straight off `triggers`
    // rather than re-deriving the predicate here: `triggers` IS that
    // predicate's own output, so this is one source of truth, not a second
    // copy that could drift. Consulted by `derive_new_data`'s key-grain
    // branch (`incremental_shapes.md` §"The key grain (`grain: key`)") to
    // waive the append-only obligation for a source maintained by a covered
    // enrichment cell instead of folded.
    let covered_by_mutation: BTreeSet<String> = triggers
        .iter()
        .filter_map(|t| match t {
            Trigger::UpstreamMutation { source } => Some(source.clone()),
            _ => None,
        })
        .collect();

    for trigger in triggers {
        match trigger {
            Trigger::NewData { source } => derive_new_data(
                inputs,
                &loc,
                source,
                &identity,
                &covered_by_mutation,
                &mut plan,
            ),
            Trigger::UpstreamMutation { source } => derive_mutation(
                inputs,
                &loc,
                source,
                &identity,
                source_referential_integrity,
                &covered_by_mutation,
                &mut plan,
            ),
            Trigger::ColumnAdded { columns } => {
                derive_column_added(inputs, &loc, columns, &identity, &mut plan)
            }
            Trigger::Backfill => derive_backfill(inputs, &loc, &identity, &mut plan),
        }
    }

    // P4 fingerprint projection (`model_properties.md` §"Fingerprint
    // projection"): a property of the model's own SQL against each
    // declared source, not of any one trigger/technique — derived once and
    // shared across every cell this model produced, mirroring how
    // `identity` above is one row-identity verdict shared by every cell.
    let projections = model_fingerprint_projections(inputs);
    if !projections.is_empty() {
        for cell in &mut plan.cells {
            cell.fingerprint_projections = projections.clone();
        }
    }

    plan
}

/// Derive the P4 fingerprint-projection verdict (`model_properties.md`
/// §"Fingerprint projection") of `inputs.sql` against every one of
/// `inputs.sources` — the column set a row-content fingerprint sidecar
/// would digest for each. Pure data; no sidecar/digest machinery here
/// (that is F3's scope).
fn model_fingerprint_projections(inputs: &ModelInputs) -> BTreeMap<String, FingerprintProjection> {
    inputs
        .sources
        .iter()
        .map(|s| (s.name.clone(), fingerprint_projection(inputs.sql, &s.name)))
        .collect()
}

/// Creation: new rows in the driving source. Partition grain recomputes the
/// new region (today's mechanism — for a pure append the RMW corner
/// degenerates to the same insert); key grain folds the delta into stored
/// key state, admitted only for a faithful additive combiner over an
/// append-only source (`01-framework.md` §4).
fn derive_new_data(
    inputs: &ModelInputs,
    loc: &LocalityInputs<'_>,
    source: &str,
    identity: &RowIdentityVerdict,
    covered_by_mutation: &BTreeSet<String>,
    plan: &mut MaintenancePlan,
) {
    let trigger = Trigger::NewData {
        source: source.to_string(),
    };
    match &inputs.output.grain {
        Grain::Partition { .. } => {
            let (partition_local, scans) = read_locality(inputs, loc);
            plan.cells.push(PlanCell {
                group: "{*}".to_string(),
                trigger,
                corner: Corner::RecomputeRegion,
                technique: Technique::DeleteInsert,
                partition_local,
                scans,
                ledger_catch_up: false,
                row_identity: identity.clone(),
                skeleton_source_closure: None,
                fingerprint_projections: BTreeMap::new(),
                key_scope: None,
            });
        }
        Grain::Key { unique_key } => {
            let Some(facts) = inputs.source(source) else {
                plan.refusals.push(Refusal::NoAdmissibleTechnique {
                    trigger: format!("{trigger:?}"),
                    why: format!("unknown source '{source}'"),
                });
                return;
            };
            let Some(fold) = &inputs.fold else {
                // No recognised fold-family column (`FoldSpec` only ever
                // carries additive/extremal/order-monotone combiners,
                // `smelt-db::queries::maintenance::derive_fold_spec`) over
                // an UNCLOCKED source, WITH a proven non-empty key (a real
                // `GROUP BY`, not a degenerate/malformed declaration), is
                // exactly the plain-overwrite/snapshot-reconcile shape
                // (`docs/specs/incremental_shapes.md` §"The two run
                // shapes"; `docs/plans/20260809-keyed-frontier.md` Phase
                // 3) — a model whose only non-key columns are
                // `ANY_VALUE(...)` (or none at all). That shape is not
                // driven by a `NewData` fold cell at all (the
                // snapshot-reconcile executor re-scans the whole source
                // every run, `smelt-runtime::cumulative::execute_
                // snapshot_reconcile`), so this trigger needs neither a
                // cell nor a refusal — silently skip, mirroring the
                // enrich-only waiver above `derive_new_data`'s obligation-2
                // narrowing. An empty `unique_key` means the model never
                // proved a real keyed grain in the first place (e.g. no
                // `GROUP BY` at all) — a genuinely different, malformed
                // shape that keeps the pre-existing refusal regardless of
                // clock; a CLOCKED source with no fold columns (but a real
                // key) is likewise the pre-existing degenerate-refusal
                // case, unaffected by this narrowing.
                if facts.partition_col.is_none() && !unique_key.is_empty() {
                    return;
                }
                plan.refusals.push(Refusal::NoAdmissibleTechnique {
                    trigger: format!("{trigger:?}"),
                    why: "keyed grain with no fold specification".to_string(),
                });
                return;
            };

            // Per-cell admission obligation 2 (`incremental_models.md`
            // §"Per-cell admission"): the faithful fold's two INDEPENDENT
            // conditions — source posture (does the delta stream partition
            // the input, i.e. is it retraction-free) and combiner algebra
            // (can a retracted contribution be undone) — either failing
            // alone refuses the fold family for this cell
            // (`model_properties.md` §"Faithful-fold conditions"). Obligation
            // 3 (combiner algebra class) is checked independently of source
            // posture: a holistic/unrecognised combiner refuses regardless of
            // how clean the source is, and leaves only the recompute family
            // admissible for this cell (no fold cell is synthesized in v0 —
            // `derive_backfill`/a declared `full` refresh is that family's
            // representative today; wiring the fallback as an alternate
            // technique inside the same cell is deferred, since v0 admits at
            // most one technique per cell). Checked per column below —
            // obligation 3 is independent per combiner, so a mixed fold
            // (e.g. `SUM` alongside `MIN`/`MAX`) refuses as a whole the
            // moment any one column's combiner fails it.

            // Obligation 2, source-posture half: `input_delta_discovery` is
            // the SC-2 tripwire's (`docs/research/property-discovery/
            // ledger.md`) production consumer. A clocked `Mutable` source's
            // `WindowForward` discovery only proves *how new rows are found*
            // — it has no branch for an in-place update to an
            // already-processed partition, so it can never by itself widen a
            // source to "retraction-free". The declared `MutationProfile`
            // remains the sole source of that fact (never derived from
            // discovery kind alone) — inside the proof, `discovery` refines
            // only the failure *reason*, never the verdict.
            let discovery = input_delta_discovery(source_shape(facts));

            // Narrowing (`incremental_shapes.md` §"The key grain
            // (`grain: key`)"), applied BEFORE the faithful-fold proof is
            // consulted — this is derive-layer waiver POLICY, not part of
            // the proof: the append-only obligation binds a
            // FOLD-CONTRIBUTING source, not every source the model
            // references. This `NewData` trigger's obligation is waived
            // iff (i) `source` is covered by an `UpstreamMutation` cell
            // for this model — its post-creation mutations are
            // maintained by that cell, not silently dropped — AND (ii)
            // `source_contributes_to_fold` proves `source` is never an
            // argument to the fold's own aggregates. Both conditions are
            // required: coverage alone would let an un-retractable
            // folded contribution through (the classifier's
            // conservatism is the safety net there); non-contribution
            // alone would still fold an un-retractable delta with
            // nothing else maintaining it. A source that is both
            // fold-contributing and mutable stays refused below — the
            // folded contribution genuinely is un-retractable. When
            // waived, this `NewData{source}` trigger needs no technique
            // at all (no cell, no refusal): `source`'s deltas do not
            // feed the fold, and its post-creation mutations are already
            // this model's `UpstreamMutation{source}` cell's job, not
            // this one's.
            if facts.mutation != MutationProfile::AppendOnly
                && covered_by_mutation.contains(source)
                && !source_contributes_to_fold(inputs.sql, source)
            {
                return;
            }

            // Obligations 2 and 3 are the two faithful-fold conditions
            // (`model_properties.md` §"Faithful-fold conditions"), derived
            // by the pure `faithful_fold` proof; this function only maps a
            // failing verdict onto the existing refusal text. Condition (1)
            // is combiner-independent, so one representative verdict decides
            // it for the whole fold (`Count` stands in when the fold has no
            // add columns — the posture obligation still binds).
            let posture = match facts.mutation {
                MutationProfile::AppendOnly => DeltaMutationProfile::AppendOnly,
                MutationProfile::MutableSnapshot => DeltaMutationProfile::Mutable,
                MutationProfile::ChangeFeed => DeltaMutationProfile::ChangeFeed,
            };
            let representative = fold
                .add_columns
                .first()
                .map(|(_, c)| *c)
                .unwrap_or(SqlFunction::Count);
            if let FaithfulFold::Fails {
                partitioned_input: ConditionVerdict::Fails { reason },
                ..
            } = faithful_fold(representative, false, &posture, discovery)
            {
                // Repair narrowing (`incremental_models.md` §"The repair
                // family"): the faithful-fold's source-posture obligation is
                // exactly the retraction case the repair family exists for —
                // before refusing outright, attempt the per-group recompute
                // technique. Repair only ever converts a refusal into a
                // cell; it never replaces an already-admitted fold, so this
                // branch (already established: the posture obligation
                // failed) is the only site it can fire from. When repair
                // admission also fails, the pre-existing
                // `NoAdmissibleTechnique` refusal is still pushed, and the
                // repair refusal is pushed alongside it naming the failing
                // obligation — additive, not a replacement.
                //
                // A `ChangeFeed` source has no fingerprint-sidecar to diff
                // (`repair::discovery_posture` has no `SidecarDiff` arm for
                // it — the feed's delta shape isn't consumed yet,
                // `incremental_models.md` §Known Divergences), so the repair
                // family is refused here, loud and named, rather than
                // attempted against a discovery posture that doesn't exist.
                if facts.mutation == MutationProfile::ChangeFeed {
                    plan.refusals.push(Refusal::NoAdmissibleTechnique {
                        trigger: format!("{trigger:?}"),
                        why: format!(
                            "fold over '{source}' fails the faithful-fold source-posture \
                             condition: {reason}, and the repair family has no fingerprint-\
                             sidecar discovery for a change_feed source",
                        ),
                    });
                    return;
                }
                let delta = repair::delta_shape_for_source(inputs.sql, facts);
                match repair::admit_per_group_recompute(
                    inputs.sql,
                    unique_key,
                    facts,
                    inputs.output_partition_col(),
                    inputs.keyed_time_axis,
                    loc,
                    &delta,
                ) {
                    Ok(admitted) => {
                        // Alphabetically sorted, matching
                        // `ColumnGroup::name()`'s own convention
                        // (`grouping::derive_column_groups` buckets columns
                        // via a `BTreeMap<String, _>` keyed by column name,
                        // never SQL declaration order) — `matching_write_pin`
                        // compares this string against a `ColumnGroup`'s own
                        // `name()` by exact equality, so a repair cell whose
                        // presented columns aren't already alphabetical in
                        // the SQL (e.g. `OrderMonotone`'s `(max_by_val,
                        // max_by_ord)`) must still agree with it.
                        let mut add_column_names: Vec<String> = fold
                            .add_columns
                            .iter()
                            .map(|(name, _)| name.clone())
                            .collect();
                        add_column_names.sort();
                        plan.cells.push(repair::derive_repair_cell(
                            &admitted,
                            trigger,
                            format!("{{{}}}", add_column_names.join(", ")),
                        ));
                    }
                    Err(refusal) => {
                        plan.refusals.push(Refusal::NoAdmissibleTechnique {
                            trigger: format!("{trigger:?}"),
                            why: format!(
                                "fold over '{source}' fails the faithful-fold source-posture \
                                 condition: {reason} (whether or not any of the fold's combiners \
                                 ({:?}) are themselves monoids)",
                                fold.add_columns.iter().map(|(_, c)| *c).collect::<Vec<_>>()
                            ),
                        });
                        plan.refusals.push(match refusal {
                            repair::RepairRefusal::KeysNotDiscoverable { source, why } => {
                                Refusal::RepairKeysNotDiscoverable { source, why }
                            }
                            repair::RepairRefusal::SliceUnbounded { source, why } => {
                                Refusal::RepairSliceUnbounded { source, why }
                            }
                        });
                    }
                }
                return;
            }

            // Condition (2): combiner algebra class, checked independently
            // of the (already-passed) source-posture condition above, per
            // column — a mixed-combiner fold refuses as a whole (fail-closed,
            // not a partial fold) the moment any one column's combiner is
            // not a monoid.
            for (column, combiner) in &fold.add_columns {
                if let FaithfulFold::Fails {
                    submultiset_fold: ConditionVerdict::Fails { reason },
                    ..
                } = faithful_fold(*combiner, false, &posture, discovery)
                {
                    // The once-write family (`COALESCE`,
                    // `incremental_shapes.md` §"The column-family
                    // catalogue") is not a commutative monoid and not
                    // order-monotone either, so this ALGEBRA leg — and only
                    // this leg — would fail-closed-refuse it. Its admission
                    // rests on an INDEPENDENT proof already verified by the
                    // SAME shared helper the runtime classifier uses
                    // (`rules::cumulative::classify_once_write`:
                    // key-derived, or a declared `key -> <source column>`
                    // functional dependency not structurally disproven —
                    // `smelt_db::queries::maintenance::derive_fold_spec`
                    // only ever puts a `Coalesce` column into a `FoldSpec`
                    // after that proof passes), so this stage does not
                    // re-derive it. The waiver is scoped to the algebra
                    // verdict: the source-posture / delta-discovery
                    // condition is combiner-independent and already binds
                    // above (the `representative` check), and the
                    // snapshot-reconcile run-shape gate below binds too.
                    if *combiner == SqlFunction::Coalesce {
                        continue;
                    }
                    plan.refusals.push(Refusal::NoAdmissibleTechnique {
                        trigger: format!("{trigger:?}"),
                        why: format!("combiner {combiner:?} for column '{column}' {reason}"),
                    });
                    return;
                }
            }
            // Run-shape gate (`docs/specs/incremental_shapes.md` §"The two
            // run shapes"; plan/classifier agreement with
            // `rules::cumulative::classify_cumulative`'s
            // `KeyedSnapshotSourceUnsupportedColumn`), consulted last —
            // AFTER both faithful-fold obligations above already passed —
            // because it is an INDEPENDENT admission axis, not a competing
            // reason for the same failure: obligations 2/3 read the
            // TRIGGERING source's declared `MutationProfile`/combiner (an
            // append-only source's posture passes obligation 2 on its own,
            // clock-independent), which cannot by itself distinguish "this
            // source's deltas are a retraction-free event feed"
            // (window-forward) from "this whole model has no clocked source
            // at all and is driven by a full-snapshot rescan every run"
            // (snapshot-reconcile, `smelt-runtime::cumulative::
            // execute_snapshot_reconcile`). The run shape is a WHOLE-MODEL
            // property — zero clocked sources anywhere in the model,
            // mirroring `CumulativeClassification::is_snapshot_reconcile`
            // — never a property of the single triggering `source`, so
            // every declared source is consulted here, not just `facts`.
            // Mirroring `classify_cumulative`'s own resolution
            // (`resolve_single_anchor`) further: snapshot-reconcile is only
            // derived when the zero-clocked sources resolve to a SINGLE
            // unambiguous candidate — two or more declared sources with
            // none clocked is the distinct `KeyedSnapshotPostureUnsupported`
            // shape (an unrelated, pre-existing refusal this gate does not
            // own), not the double-count case this gate names. A model
            // already refused above (e.g. a `MutableSnapshot` source
            // failing obligation 2's posture check) keeps ITS refusal
            // reason — this gate only fires for a fold that would
            // otherwise be admitted, where under snapshot-reconcile it
            // always double-counts (or, for an extremal/order-monotone
            // combiner, computes a history observation instead of the
            // current value) no matter how clean the triggering source's
            // own posture is. Refused fail-loud, not silently skipped like
            // the no-fold arm above (a real fold specification WAS found;
            // admitting nothing without saying why would hide the gap from
            // `smelt explain`).
            if inputs.sources.len() == 1 && inputs.sources[0].partition_col.is_none() {
                plan.refusals.push(Refusal::NoAdmissibleTechnique {
                    trigger: format!("{trigger:?}"),
                    why: format!(
                        "fold over '{source}' is refused under the snapshot-reconcile run \
                         shape (no clocked source anywhere in this model) — re-folding state \
                         double-counts: a mutable snapshot is not a replayable, \
                         retraction-free event feed (or, for an extremal/order-monotone \
                         combiner, computes a history observation instead of the current \
                         value). Wrap the fold column as ANY_VALUE(...) for the \
                         plain-overwrite family instead, or declare `timeseries:` on a \
                         driving source to use the window-forward run shape."
                    ),
                });
                return;
            }

            plan.cells.push(PlanCell {
                group: format!(
                    "{{{}}}",
                    fold.add_columns
                        .iter()
                        .map(|(name, _)| name.clone())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                trigger,
                corner: Corner::FoldDelta,
                technique: Technique::KeyedFold,
                // Keyed end-state: the write is key-addressed, not
                // partition-addressed; there is no partition axis to bound.
                partition_local: PartitionLocal::Yes,
                scans: vec![],
                ledger_catch_up: false,
                row_identity: identity.clone(),
                skeleton_source_closure: None,
                fingerprint_projections: BTreeMap::new(),
                key_scope: None,
            });
        }
    }
}

/// Mutation: a post-creation delta in `source` touches exactly the column
/// groups mutation-sensitive to it — the bottom-left column-scoped
/// re-derivation. Partition-local only when the source's partition column is
/// explicitly linked to the output axis (K8's ratified
/// `require: partition_local` refuses the unlinked/unclocked case unless the
/// full scan is declared).
///
/// `source_referential_integrity` is [`mutation_enrichment_closure`]'s own
/// input, threaded straight through — see that function's doc comment for
/// the `None`-vs-attempted-and-`Open` distinction this preserves.
fn derive_mutation(
    inputs: &ModelInputs,
    loc: &LocalityInputs<'_>,
    source: &str,
    identity: &RowIdentityVerdict,
    source_referential_integrity: &SourceReferentialIntegrity,
    covered_by_mutation: &BTreeSet<String>,
    plan: &mut MaintenancePlan,
) {
    let trigger = Trigger::UpstreamMutation {
        source: source.to_string(),
    };
    let Some(facts) = inputs.source(source) else {
        plan.refusals.push(Refusal::NoAdmissibleTechnique {
            trigger: format!("{trigger:?}"),
            why: format!("unknown source '{source}'"),
        });
        return;
    };

    // P1 skeleton-source closure (`model_properties.md` §"Skeleton-source
    // closure"; T3 over external sources, `docs/plans/20260715-composed-
    // axes-conditional-maintenance.md` Phase F5): a property of this cell's
    // own enrichment join against `source`, derived once and shared by
    // every column-group cell this source drives — mirroring
    // `append_model_edge_cells`'s `model_edge_enrichment_closure`, the
    // model-edge analogue this generalizes (the "licence union" the phase
    // wires: the SAME `skeleton_source_closure` proof and the SAME
    // `choice::resolve_recompute_restriction` gate now admit an external
    // mutable-snapshot source's enrichment join, not only a model edge's).
    let closure = mutation_enrichment_closure(
        inputs.sql,
        source,
        &inputs.sources,
        source_referential_integrity,
    );

    for group in inputs.column_groups.iter().filter(|g| {
        g.mutation_sensitivity.contains(source) || g.membership_sensitivity.contains(source)
    }) {
        // A `Trigger::NewData { source }` repair cell (`Technique::
        // PerGroupRecompute`, `incremental_models.md` §"The repair family")
        // already recomputes this exact group's bounded, per-key-affected
        // scope every run — including a value change to an already-created
        // row, the SAME thing an `UpstreamMutation` cell would exist to
        // catch. Deriving both would double-write the same group from the
        // same source in the same run (phase 19,
        // `docs/outcomes/20260815-definition-delta-migrate`: widening
        // `UpstreamMutation` derivation to a clocked `MutableSnapshot`
        // source newly makes this collision reachable — the repair family
        // is only ever admitted for exactly the retracting-source shape
        // this trigger also now covers). `triggers` always orders a
        // source's `NewData` before its `UpstreamMutation`
        // (`derive_triggers`), so the repair cell, if any, is already in
        // `plan.cells` by the time this loop runs.
        let already_repaired = plan.cells.iter().any(|c| {
            c.group == group.name()
                && c.technique == Technique::PerGroupRecompute
                && matches!(&c.trigger, Trigger::NewData { source: s } if s == source)
        });
        if already_repaired {
            continue;
        }
        // Membership sensitivity (`docs/specs/incremental_models.md` §"The
        // plan matrix"): a group governed by a row-admission read of
        // `source` must be repaired by a technique that can create and
        // delete rows — the recompute family — even when `source` also
        // happens to be pure value-sensitive for this same group. Only a
        // group whose sensitivity to `source` is *purely* value (never
        // membership) is eligible for the cheaper column-scoped merge.
        let membership_sensitive = group.membership_sensitivity.contains(source);
        let (locality, scans) = match project_source_link(
            inputs.output_partition_col(),
            inputs.keyed_time_axis,
            loc,
            facts,
        ) {
            SourceLink::Clamp(clamp) => (PartitionLocal::Yes, vec![clamp]),
            SourceLink::Unclocked => (
                PartitionLocal::No {
                    source: source.to_string(),
                    why: "unclocked source: a change's footprint projects onto no bounded \
                          partition interval of the output"
                        .to_string(),
                },
                vec![],
            ),
            SourceLink::Unlinked { why } => (
                PartitionLocal::No {
                    source: source.to_string(),
                    why,
                },
                vec![],
            ),
        };
        if matches!(locality, PartitionLocal::No { .. }) && !facts.allow_full_scan {
            plan.refusals.push(Refusal::ScanUnbounded {
                source: source.to_string(),
                why: format!(
                    "maintenance of {} driven by '{source}' scatters across all output \
                     partitions; declare allow_full_scan to accept the full-table write",
                    group.name()
                ),
            });
            continue;
        }
        // Group-merge-provenance guard (`incremental_models.md` §"The plan
        // matrix", decided in success criterion 18's "Group-merge-provenance
        // policy" open question): a group whose sensitivity spans TWO OR
        // MORE mutation-capable inputs is repaired by region recompute,
        // never a column-scoped merge — the conservative, always-correct
        // default every other mutation-sensitivity rule here already takes.
        // "Mutation-capable" means the source actually gets its own
        // `UpstreamMutation` trigger (`covered_by_mutation`, read straight
        // off `triggers` — the SAME predicate `derive_triggers` already
        // computed, not a second guess): an append-only source with no
        // value-sensitivity of its own, or one this model doesn't derive a
        // mutation cell for, does not count toward the merge, matching
        // `membership_sensitive`'s existing scoping to genuinely
        // mutation-driven groups.
        let mutation_capable_inputs = group
            .mutation_sensitivity
            .union(&group.membership_sensitivity)
            .filter(|s| covered_by_mutation.contains(*s))
            .count();
        // A `ChangeFeed` source's cell is clamped to full-input
        // re-derivation, the same forcing shape the merged-group guard
        // above uses: no column-scoped merge or fold realisation exists
        // for a posture whose delta shape is never read
        // (`incremental_models.md` §Known Divergences).
        let (corner, technique) = if facts.mutation == MutationProfile::ChangeFeed
            || membership_sensitive
            || mutation_capable_inputs >= 2
        {
            (Corner::RecomputeRegion, Technique::DeleteInsert)
        } else {
            (Corner::ColumnMerge, Technique::ColumnScopedMerge)
        };
        plan.cells.push(PlanCell {
            group: group.name(),
            trigger: trigger.clone(),
            corner,
            technique,
            partition_local: locality,
            scans,
            ledger_catch_up: false,
            row_identity: identity.clone(),
            skeleton_source_closure: closure.clone(),
            fingerprint_projections: BTreeMap::new(),
            key_scope: None,
        });
    }
}

/// Extract one named column's [`ColumnDef`] (name + defining expression)
/// from `sql`'s own outermost `SELECT` scope — a `ColumnAdded` trigger fires
/// because `name` already exists in the model's current `sql`, so this
/// resolves the [`crate::analysis::definition_change::classify_definition_
/// change`] proof's `added_column` argument straight from the same source
/// the rest of this derivation already reads. `None` when `sql` has no
/// classifiable top-level `SELECT`, or `name` isn't one of its projected
/// aliases — the caller fails closed rather than guessing an expression.
pub fn column_def_from_sql(sql: &str, name: &str) -> Option<ColumnDef> {
    let stripped = crate::types::Frontmatter::strip(sql);
    let parse = smelt_parser::parse(stripped);
    let file = smelt_parser::File::cast(parse.syntax())?;
    let select = file.select_stmt()?;
    let items = select_stmt_items(&select)?;
    items
        .iter()
        .find(|item| item_alias(item) == name)
        .map(|item| ColumnDef {
            name: name.to_string(),
            expr: item_expr(item).clone(),
        })
}

/// Diff `sql`'s own currently-projected output columns against
/// `deployed_column_names` (a prior deployed-schema snapshot's column
/// names, world-fact supplied by the caller — this function does no I/O of
/// its own, per the Salsa-purity/plan-purity rule) to derive the two
/// ingredients a production `Trigger::ColumnAdded` needs:
///
/// - `old_columns` — every currently-projected column whose name is ALSO
///   in `deployed_column_names` (i.e. still-present, pre-existing output
///   columns), with its defining expression read from `sql` itself. This
///   is [`ModelInputs::old_columns`] — [`classify_definition_change`]'s
///   `ctx.old_columns` (leg 2's collision check, leg 3's "already stored"
///   set).
/// - `added` — every currently-projected column name that is NOT in
///   `deployed_column_names`: the genuinely new columns a `Trigger::
///   ColumnAdded { columns: added }` should carry.
///
/// `None` when `sql` has no classifiable top-level `SELECT` — the caller
/// fails closed (no trigger derived) rather than guessing, exactly like
/// [`column_def_from_sql`] above.
pub fn diff_deployed_columns(
    sql: &str,
    deployed_column_names: &[String],
) -> Option<(Vec<ColumnDef>, Vec<String>)> {
    let stripped = crate::types::Frontmatter::strip(sql);
    let parse = smelt_parser::parse(stripped);
    let file = smelt_parser::File::cast(parse.syntax())?;
    let select = file.select_stmt()?;
    let items = select_stmt_items(&select)?;
    let deployed: std::collections::HashSet<&str> =
        deployed_column_names.iter().map(|s| s.as_str()).collect();
    let mut old_columns = Vec::new();
    let mut added = Vec::new();
    for item in &items {
        let name = item_alias(item);
        if deployed.contains(name) {
            old_columns.push(ColumnDef {
                name: name.to_string(),
                expr: item_expr(item).clone(),
            });
        } else {
            added.push(name.to_string());
        }
    }
    Some((old_columns, added))
}

/// Prove (or refuse to prove) that `inputs.sql`'s skeleton clause — the
/// FROM/JOIN tree, GROUP BY/DISTINCT, and the row-set/ordering/row-count
/// post-processing clauses — is unchanged against `inputs.old_sql`, using
/// the same clause-level factoring `smelt migrate` uses
/// ([`crate::backbuild::diff::definition_diff`]). Returns `Some(reason)`
/// when [`crate::backbuild::SkeletonDiff::Changed`] proves a clause
/// difference — the `Refusal::SkeletonClauseChanged` this function's caller
/// pushes. Returns `None` when there is no deployed SQL to compare
/// (`inputs.old_sql` is `None`), when either version fails to parse as a
/// plain `SELECT`, or when the diff proves the skeleton
/// [`crate::backbuild::SkeletonDiff::Unchanged`] or only gained `LEFT
/// JOIN`s (`AddedLeftJoins`, admissible — research §4's G0 class). A
/// `DefinitionDiff::Opaque` diff (unparseable, or a changed `WITH`-prefix)
/// is deliberately NOT treated as a skeleton change here — this check only
/// ever *adds* a refusal on a positive proof, never on an inability to
/// prove either way, matching every other `deployed_*`-gated derivation in
/// this module.
fn skeleton_clause_changed(inputs: &ModelInputs) -> Option<String> {
    let old_sql = inputs.old_sql?;
    let old_stripped = crate::types::Frontmatter::strip(old_sql);
    let new_stripped = crate::types::Frontmatter::strip(inputs.sql);
    let old_parse = smelt_parser::parse(old_stripped);
    let new_parse = smelt_parser::parse(new_stripped);
    let old_file = smelt_parser::File::cast(old_parse.syntax())?;
    let new_file = smelt_parser::File::cast(new_parse.syntax())?;
    match crate::backbuild::diff::definition_diff(&old_file, &new_file) {
        crate::backbuild::DefinitionDiff::Comparable(diff) => match diff.skeleton {
            crate::backbuild::SkeletonDiff::Changed { reason, .. } => Some(reason),
            crate::backbuild::SkeletonDiff::Unchanged
            | crate::backbuild::SkeletonDiff::AddedLeftJoins(_) => None,
        },
        crate::backbuild::DefinitionDiff::Opaque { .. } => None,
    }
}

/// Definition change: the model gained fields. Skeleton adds are grain
/// changes and refuse (EX-39); payload adds land in the 2×2's left column by
/// what they read (EX-36/37/40), instantiating their ledger entries at
/// `S = ∅` (the catch-up flag).
fn derive_column_added(
    inputs: &ModelInputs,
    loc: &LocalityInputs<'_>,
    columns: &[String],
    identity: &RowIdentityVerdict,
    plan: &mut MaintenancePlan,
) {
    let trigger = Trigger::ColumnAdded {
        columns: columns.to_vec(),
    };
    // Boundary first: a skeleton-position add changes which rows exist.
    // Two independent proofs must both clear every added column before
    // *either* branch below (empty-sensitivity in-place update, or the
    // mutation-sensitive column-scoped merge) is allowed to dispatch it:
    // the hand-declared `output.skeleton_columns` set, and — when the
    // model's current SQL is classifiable — the derived skeleton-role
    // extraction (`maintenance::skeleton::skeleton_roles`). Only the
    // declared-set check used to run here; the derived check ran solely
    // inside `classify_definition_change`, which the non-empty-sensitivity
    // branch never calls, so a `GROUP BY` key absent from the declared set
    // could reach `ColumnScopedMerge` in that branch undetected. Computing
    // `skeleton_roles` once here, ahead of both branches, closes that gap.
    // An unclassifiable shape (`skeleton_roles` returns `None`) does not
    // newly refuse a model that relied on the declared set alone —
    // fail-closed without over-refusing.
    let derived_roles = skeleton_roles(
        inputs.sql,
        inputs.declared_unique_key(),
        inputs.output_partition_col(),
    );
    for col in columns {
        if inputs.output.skeleton_columns.contains(col) {
            plan.refusals.push(Refusal::SkeletonChanged {
                column: col.clone(),
            });
            return;
        }
        if let Some(role) = derived_roles
            .as_ref()
            .and_then(|roles| roles.iter().find(|(name, _)| name == col))
            .map(|(_, role)| *role)
        {
            if role.is_skeleton() {
                plan.refusals.push(Refusal::SkeletonChanged {
                    column: col.clone(),
                });
                return;
            }
        }
    }

    // The added fields factor by shared mutation-sensitivity exactly as the
    // base plan does; each added group gets its own catch-up op.
    for group in inputs
        .column_groups
        .iter()
        .filter(|g| g.columns.iter().any(|c| columns.contains(c)))
    {
        if group.mutation_sensitivity.is_empty() {
            // Empty mutation-sensitivity is *eligible* for the cheap
            // in-place update, but is not on its own proof of purity — it
            // can also mean "an append-only source read that is never
            // re-mutated after creation, yet was never stored before" (the
            // misclassification `classify_definition_change` exists to
            // correct, `model_properties.md` §"Definition-change column
            // classification"). Every added column in this group is run
            // through the composed proof; a `PureBackfill` verdict admits
            // the in-place `UPDATE`, a `SkeletonAdd` verdict this group's
            // hand-declared `output.skeleton_columns` didn't already catch
            // still refuses as a grain change, and an `UpstreamRederive`
            // verdict — or any refusal — fails closed here: this group
            // carries no source name to scan (that is exactly what "empty
            // mutation-sensitivity" means), so a column-scoped merge cannot
            // be constructed from the inputs this v0 derivation has
            // (production `ColumnAdded` trigger derivation, which could
            // supply one, stays out of this phase's scope).
            let added_in_group: Vec<&String> = group
                .columns
                .iter()
                .filter(|c| columns.contains(c))
                .collect();
            let mut verdict: Option<DefinitionChangeClass> = None;
            let mut refused: Option<String> = None;
            'columns: for c in &added_in_group {
                let Some(def) = column_def_from_sql(inputs.sql, c) else {
                    refused = Some(format!(
                        "could not resolve '{c}''s expression in the model's own SQL"
                    ));
                    break 'columns;
                };
                let ctx = DefinitionChangeCtx {
                    old_columns: &inputs.old_columns,
                    declared_unique_key: inputs.declared_unique_key(),
                    partition_col: inputs.output_partition_col(),
                    declared_skeleton_columns: &inputs.output.skeleton_columns,
                    monotone_dims: &[],
                };
                match classify_definition_change(&def, inputs.sql, &ctx) {
                    Ok(DefinitionChangeClass::SkeletonAdd { .. }) => {
                        plan.refusals.push(Refusal::SkeletonChanged {
                            column: (*c).clone(),
                        });
                        return;
                    }
                    Ok(v) => match &verdict {
                        None => verdict = Some(v),
                        Some(prev) if *prev == v => {}
                        Some(_) => {
                            refused = Some(
                                "group columns disagree on definition-change classification"
                                    .to_string(),
                            );
                            break 'columns;
                        }
                    },
                    Err(e) => {
                        refused = Some(format!("{e:?}"));
                        break 'columns;
                    }
                }
            }
            match (verdict, refused) {
                (Some(DefinitionChangeClass::PureBackfill), None) => {
                    plan.cells.push(PlanCell {
                        group: group.name(),
                        trigger: trigger.clone(),
                        corner: Corner::FoldDelta,
                        technique: Technique::InPlaceUpdate,
                        partition_local: PartitionLocal::Yes,
                        scans: vec![],
                        ledger_catch_up: true,
                        row_identity: identity.clone(),
                        skeleton_source_closure: None,
                        fingerprint_projections: BTreeMap::new(),
                        key_scope: None,
                    });
                }
                (Some(DefinitionChangeClass::UpstreamRederive), None) => {
                    plan.refusals
                        .push(Refusal::DefinitionChangeNotBackfillable {
                            columns: added_in_group.iter().map(|c| (*c).clone()).collect(),
                            why: "re-derives from upstream, but this group's mutation-sensitivity \
                              names no source to scan — a column-scoped merge cannot be \
                              constructed"
                                .to_string(),
                        });
                }
                (_, Some(why)) => {
                    plan.refusals
                        .push(Refusal::DefinitionChangeNotBackfillable {
                            columns: added_in_group.iter().map(|c| (*c).clone()).collect(),
                            why,
                        });
                }
                (_, None) => {
                    plan.refusals
                        .push(Refusal::DefinitionChangeNotBackfillable {
                            columns: added_in_group.iter().map(|c| (*c).clone()).collect(),
                            why: "in-place update not proven additive-only".to_string(),
                        });
                }
            }
            continue;
        }

        // Re-derives from upstream: column-scoped MERGE. Every read source
        // must be linked to the output partition axis or explicitly accepted
        // as a full read (EX-38: the field-add inherits its source's
        // partition-locality verdict unchanged).
        let mut scans = Vec::new();
        let mut locality = PartitionLocal::Yes;
        let mut refused = false;
        for source_name in &group.mutation_sensitivity {
            let Some(facts) = inputs.source(source_name) else {
                plan.refusals.push(Refusal::NoAdmissibleTechnique {
                    trigger: format!("{trigger:?}"),
                    why: format!("unknown source '{source_name}'"),
                });
                refused = true;
                break;
            };
            match project_source_link(
                inputs.output_partition_col(),
                inputs.keyed_time_axis,
                loc,
                facts,
            ) {
                SourceLink::Clamp(clamp) => scans.push(clamp),
                SourceLink::Unclocked | SourceLink::Unlinked { .. } if !facts.allow_full_scan => {
                    plan.refusals
                        .push(Refusal::DefinitionChangeNotBackfillable {
                            columns: group
                                .columns
                                .iter()
                                .filter(|c| columns.contains(c))
                                .cloned()
                                .collect(),
                            why: format!(
                                "backfill of {} reads '{}' with no partition bound",
                                group.name(),
                                facts.name
                            ),
                        });
                    refused = true;
                    break;
                }
                SourceLink::Unclocked => {
                    locality = PartitionLocal::No {
                        source: facts.name.clone(),
                        why: "unclocked source read in full (declared)".to_string(),
                    };
                }
                SourceLink::Unlinked { why } => {
                    locality = PartitionLocal::No {
                        source: facts.name.clone(),
                        why: format!("{why} (declared full scan)"),
                    };
                }
            }
        }
        if refused {
            continue;
        }
        plan.cells.push(PlanCell {
            group: group.name(),
            trigger: trigger.clone(),
            corner: Corner::ColumnMerge,
            technique: Technique::ColumnScopedMerge,
            partition_local: locality,
            scans,
            ledger_catch_up: true,
            row_identity: identity.clone(),
            skeleton_source_closure: None,
            fingerprint_projections: BTreeMap::new(),
            key_scope: None,
        });
    }
}

/// Backfill: the universal ground-truth reset — recompute the region from
/// replayable input, unconditionally correct (`01-framework.md` §3).
fn derive_backfill(
    inputs: &ModelInputs,
    loc: &LocalityInputs<'_>,
    identity: &RowIdentityVerdict,
    plan: &mut MaintenancePlan,
) {
    let (partition_local, scans) = read_locality(inputs, loc);
    plan.cells.push(PlanCell {
        group: "{*}".to_string(),
        trigger: Trigger::Backfill,
        corner: Corner::RecomputeRegion,
        technique: Technique::DeleteInsert,
        partition_local,
        scans,
        ledger_catch_up: false,
        row_identity: identity.clone(),
        skeleton_source_closure: None,
        fingerprint_projections: BTreeMap::new(),
        key_scope: None,
    });
}

/// Partition-locality of a whole-row recompute's *reads*, plus the derived
/// scan clamps for the sources that are linked. The first unlinked or
/// unclocked source decides the `No` verdict (backfill stays admitted — a
/// recompute is the universal ground-truth reset — but the full read is
/// named, never silent).
fn read_locality(
    inputs: &ModelInputs,
    loc: &LocalityInputs<'_>,
) -> (PartitionLocal, Vec<ScanClamp>) {
    // Keyed grain: a whole-table rebuild over a key-addressed output — there
    // is no output partition axis to be local to, so the locality question
    // is not posed and the sentinel `PartitionLocal::Yes` records the
    // vacuous verdict (policy, not a proof outcome — the locality proof is
    // only ever consulted against a partition-addressed output;
    // `model_properties.md` §Known Divergences keeps this keyed residue).
    if inputs.output_partition_col().is_none() {
        return (PartitionLocal::Yes, vec![]);
    }
    let mut scans = Vec::new();
    let mut verdict = PartitionLocal::Yes;
    for s in &inputs.sources {
        match project_source_link(
            inputs.output_partition_col(),
            inputs.keyed_time_axis,
            loc,
            s,
        ) {
            SourceLink::Clamp(clamp) => scans.push(clamp),
            SourceLink::Unclocked => {
                if matches!(verdict, PartitionLocal::Yes) {
                    verdict = PartitionLocal::No {
                        source: s.name.clone(),
                        why: "unclocked source is read in full on every recompute".to_string(),
                    };
                }
            }
            SourceLink::Unlinked { why } => {
                if matches!(verdict, PartitionLocal::Yes) {
                    verdict = PartitionLocal::No {
                        source: s.name.clone(),
                        why,
                    };
                }
            }
        }
    }
    (verdict, scans)
}

/// Convenience used by tests: the set of column names across `groups`.
pub fn group_columns(groups: &[ColumnGroup]) -> BTreeSet<String> {
    groups
        .iter()
        .flat_map(|g| g.columns.iter().cloned())
        .collect()
}

#[cfg(test)]
mod source_shape_tests {
    use super::*;

    /// Phase 28c: `source_shape` maps the plan-layer `MutationProfile::
    /// ChangeFeed` onto the analysis-layer `DeltaMutationProfile::
    /// ChangeFeed` 1:1, the same mapping `AppendOnly`/`MutableSnapshot`
    /// already get — closing the "no `ChangeFeed` in the plan layer" gap
    /// this function's own doc comment used to record.
    #[test]
    fn change_feed_source_shape_maps_to_change_feed_delta_profile() {
        let facts = SourceFacts {
            name: "feed".to_string(),
            mutation: MutationProfile::ChangeFeed,
            partition_col: None,
            unique_key: vec![],
            allow_full_scan: false,
        };
        let shape = source_shape(&facts);
        assert_eq!(
            shape.mutation_profile,
            Some(DeltaMutationProfile::ChangeFeed)
        );
    }
}
