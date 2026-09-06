use super::*;

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
///
/// `sources` and `source_referential_integrity` are the SAME external-source
/// facts `derive_maintenance_plan_with_referential_integrity` threads into
/// `mutation_enrichment_closure` — folded here too so a model-edge cell's P1
/// AND covers every enrichment relation actually joined in the scope, not
/// only its upstream model edges (`model_properties.md` §"Skeleton-source
/// closure").
pub fn append_model_edge_cells(
    plan: &mut MaintenancePlan,
    sql: &str,
    output_partition_col: Option<&str>,
    model_edges: &[ModelEdge],
    declared_unique_key: &[String],
    sources: &[SourceFacts],
    source_referential_integrity: &SourceReferentialIntegrity,
) {
    if model_edges.is_empty() {
        return;
    }
    // The `JoinContext` built from every joined edge's own declared
    // `unique_key` (see `model_edges_join_context`'s doc comment), unioned
    // with the same declared-`unique_key` facts `source_facts_join_context`
    // builds for external sources — shared by the row-identity proof below
    // AND `model_edge_enrichment_closure`'s P1 proof, so both properties of
    // this SAME model-edge cell see the SAME declared facts rather than the
    // row-identity proof working from a second, independent context.
    let join_ctx =
        model_edges_join_context(sql, model_edges).union(source_facts_join_context(sql, sources));
    let identity = row_identity_with_context(declared_unique_key, sql, &join_ctx);
    // P1 skeleton-source closure — a property of the model's own query
    // shape (`model_edge_enrichment_closure`'s doc comment below), so it is
    // shared by both the key-addressed loop right below and the
    // partition-addressed loop further down, computed once.
    let enrichment_closure = model_edge_enrichment_closure(
        sql,
        model_edges,
        sources,
        source_referential_integrity,
        &join_ctx,
    );

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
                    state_downgrade: None,
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
            state_downgrade: None,
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

    let mut ctx = JoinContext::new(); // join-context: builder
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

/// Derive the shared P1 skeleton-source-closure verdict for a model-edge
/// cell — an AND over every enrichment relation actually joined in the
/// scope: each of `model_edges` judged with no declared referential-
/// integrity fact of its own (a model edge licenses row preservation only
/// via join shape, never a declaration), and each of `sources` actually
/// joined in the same scope judged with `source_referential_integrity`'s
/// declared fact for it, mirroring [`mutation_enrichment_closure`]'s own
/// per-source `skeleton_source_closure` call exactly — the SAME proof, the
/// SAME declared facts, just folded into one shared verdict instead of one
/// per `UpstreamMutation` cell (`model_properties.md` §"Skeleton-source
/// closure"). `join_ctx` is [`append_model_edge_cells`]'s unioned context —
/// the same one the caller also feeds to the row-identity proof, never a
/// second, independently-built context.
///
/// `None` when neither a model edge nor an external source is actually
/// joined in this scope — no enrichment relation to close over at all,
/// matching [`PlanCell::skeleton_source_closure`]'s documented empty case.
fn model_edge_enrichment_closure(
    sql: &str,
    model_edges: &[ModelEdge],
    sources: &[SourceFacts],
    source_referential_integrity: &SourceReferentialIntegrity,
    join_ctx: &JoinContext,
) -> Option<crate::analysis::skeleton_closure::SkeletonSourceClosure> {
    use crate::analysis::skeleton_closure::{enrichment_join_alias, skeleton_source_closure};

    let joined_edges: Vec<&ModelEdge> = model_edges
        .iter()
        .filter(|edge| enrichment_join_alias(sql, &edge.name).is_some())
        .collect();
    let joined_sources: Vec<&SourceFacts> = sources
        .iter()
        .filter(|facts| enrichment_join_alias(sql, &facts.name).is_some())
        .collect();
    if joined_edges.is_empty() && joined_sources.is_empty() {
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
    if verdict.is_closed() {
        for facts in joined_sources {
            let ri = source_referential_integrity
                .get(&facts.name)
                .map(Vec::as_slice);
            let v = skeleton_source_closure(sql, &facts.name, ri, join_ctx);
            if !v.is_closed() {
                verdict = v;
                break;
            }
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
pub(super) fn source_facts_join_context(sql: &str, sources: &[SourceFacts]) -> JoinContext {
    use crate::analysis::skeleton_closure::enrichment_join_alias;

    let mut ctx = JoinContext::new(); // join-context: builder
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
pub(super) fn mutation_enrichment_closure(
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
