use super::*;

/// The per-workspace facts [`build_forward_graph`] derives from every
/// model's `MaintenancePlan` in one pass: the widest scan-clamp margin seen
/// per `(upstream, downstream)` edge, and the key-temporal-locality
/// admission verdict for every `grain: key` model this workspace derives a
/// plan for. Factored out of `build_forward_graph` so
/// [`refuse_bare_keyed_origins`] can consult the SAME admission verdicts —
/// never re-deriving them — without threading a new field through
/// `build_forward_graph`'s own `Vec<Edge>` return type (which several
/// existing callers, including tests, already destructure directly).
pub(super) struct ClampAndLocality {
    pub(super) clamp_seconds: BTreeMap<(String, String), (i64, i64)>,
    /// The derived write footprint in whole days per `(upstream, downstream)`
    /// pair, folded the same widen-never-narrow way `clamp_seconds` is —
    /// `Some((before, after))` widened across every contributing
    /// `ScanClamp::write_footprint`, downgrading to `None` the moment any
    /// contributing cell for that pair carried no derived footprint at all
    /// (`ScanClamp::write_footprint`'s own doc comment: absence is a claim
    /// too, never silently patched over by a sibling cell's derived number).
    pub(super) footprint_seconds: BTreeMap<(String, String), Option<(i64, i64)>>,
    pub(super) locality_admitted: BTreeMap<String, bool>,
    /// The established [`smelt_logical::maintenance::locality::LocalitySlice`]
    /// route for every `grain: key` model this workspace admits key temporal
    /// locality for (Phase D3,
    /// `docs/plans/20260715-composed-axes-conditional-maintenance.md` —
    /// "Key→partition projection of observed deltas"). Consulted by
    /// [`plan_since_upstream_with_observed_deltas`] to project a recorded
    /// observed delta to exact partition dirt via
    /// `smelt_logical::maintenance::propagate::project_observed_delta` —
    /// never re-derived; the SAME verdict `locality_admitted` above folds
    /// `Some(_).is_some()` from.
    pub(super) key_locality_slice:
        BTreeMap<String, smelt_logical::maintenance::locality::LocalitySlice>,
    /// The SAME admitted verdict's own derived [`SettleBound`]
    /// (`smelt_logical::maintenance::KeyLocality::settle_bound`) — carried
    /// alongside `key_locality_slice`, never re-derived
    /// (`smelt_logical::maintenance::locality::settle_bound` is the single
    /// derivation). Consulted by [`plan_since_upstream_with_observed_deltas`]
    /// to compose a present-and-empty recorded delta with the model's
    /// settle bound (`docs/specs/incremental_models.md` §"Observed deltas
    /// on model edges" — "This composes with the derived settle bound").
    pub(super) key_locality_settle_bound:
        BTreeMap<String, smelt_logical::maintenance::locality::SettleBound>,
}

pub(super) fn derive_clamp_and_locality(
    models: &[ModelFile],
    source_infos: &[SourceInfo],
) -> Result<ClampAndLocality> {
    // Upstream maintained-model composed outputs admitted so far, keyed by
    // model address (the same key `model_edges` names an upstream by) —
    // mirrors `smelt-db::lib.rs`'s `ref_model_source_facts`/
    // `model_source_granularities` handling: a `grain: key` model whose
    // driving source is ANOTHER maintained model's own admitted composed
    // output must see that upstream as both a `SourceFacts` candidate
    // (`derive_new_data`'s `inputs.source(source)` lookup, which a bare
    // `ModelEdge` never populates — model edges only ever clamp a
    // **partition**-addressed downstream's creation cell,
    // `append_model_edge_cells`'s `output_partition_col` early return) and a
    // clocked-granularity candidate for the locality gate's own structural
    // precondition, not only declared `sources.*` refs
    // (`docs/plans/20260719-prod-w8-composed-axes-followups.md` Phase 6).
    // `smelt-db`'s Salsa queries resolve this recursively for free via
    // memoized recursion; this call site has no query-recursion to lean on,
    // so it re-runs the whole per-model pass to a fixed point instead —
    // each pass can only add a candidate an upstream model's OWN admission
    // resolved in the previous pass, so a chain of N maintained models
    // converges within N passes (never re-deriving admission itself, only
    // widening which already-derived verdicts are visible as candidates).
    //
    // This convergence argument assumes an acyclic model-ref graph. This
    // call site runs before `DependencyGraph::execution_order()` (the real
    // cycle detector) on at least one call path, and `build_forward_graph`
    // itself only rejects a literal self-reference — not a longer cycle.
    // A composed-source candidate can also *remove* an admission (flipping
    // `single_clocked_granularity` from unambiguous to ambiguous), so
    // monotonicity isn't guaranteed on a cyclic graph either — a naive
    // unbounded loop could hang, or oscillate with a period the consecutive-
    // state equality check below wouldn't catch. Bound the loop at
    // `models.len() + 1` passes (enough for the documented N-model
    // convergence argument plus one confirmation pass) and fail loud rather
    // than hang, per root `CLAUDE.md` §"Fail-loud discipline".
    let max_passes = models.len() + 1;
    let mut composed_sources: BTreeMap<String, (SourceFacts, Granularity)> = BTreeMap::new();
    // The workspace's output-delta verdicts, derived once — the SAME map
    // `build_forward_graph`'s own `type_edge` call reads
    // (`workspace_output_delta_verdicts`), so a model edge's `output_shape`
    // (below) is never a second, independent derivation of the same fact.
    let workspace_verdicts = workspace_output_delta_verdicts(models, source_infos);

    for _pass in 0..max_passes {
        let ClampAndLocality {
            clamp_seconds,
            footprint_seconds,
            locality_admitted,
            key_locality_slice,
            key_locality_settle_bound,
        } = derive_clamp_and_locality_pass(
            models,
            source_infos,
            &composed_sources,
            &workspace_verdicts,
        )?;

        let mut next_composed_sources: BTreeMap<String, (SourceFacts, Granularity)> =
            BTreeMap::new();
        for (addr, admitted) in &locality_admitted {
            if !admitted {
                continue;
            }
            let Some(slice) = key_locality_slice.get(addr) else {
                continue;
            };
            let Some(Some(granularity)) = models
                .iter()
                .find(|m| &m.canonical_path() == addr)
                .and_then(|m| m.metadata.as_deref())
                .map(|m| m.timeseries.as_ref().map(|t| t.granularity))
            else {
                continue;
            };
            let facts = SourceFacts {
                name: addr.clone(),
                // Mirrors `smelt-db::lib.rs::ref_model_source_facts`: a
                // composed maintained output's rows, once written, are not
                // retroactively mutated by a later run touching a different
                // slice — the same append-only posture a declared
                // `timeseries:` source with no explicit `mutation_profile:
                // mutable` gets by default.
                mutation: PlanMutationProfile::AppendOnly,
                partition_col: Some(slice.partition_column().to_string()),
                unique_key: Vec::new(),
                allow_full_scan: false,
            };
            next_composed_sources.insert(addr.clone(), (facts, granularity));
        }

        // `SourceFacts` carries no `PartialEq` — compare the
        // (name, partition_col, granularity) signature convergence tracks
        // by instead of the whole struct.
        let sig = |m: &BTreeMap<String, (SourceFacts, Granularity)>| {
            m.iter()
                .map(|(addr, (facts, g))| (addr.clone(), facts.partition_col.clone(), *g))
                .collect::<Vec<_>>()
        };
        if sig(&next_composed_sources) == sig(&composed_sources) {
            return Ok(ClampAndLocality {
                clamp_seconds,
                footprint_seconds,
                locality_admitted,
                key_locality_slice,
                key_locality_settle_bound,
            });
        }
        composed_sources = next_composed_sources;
    }

    bail!(
        "MaintenanceGraphUnsupportedNode: the composed-source fixed-point derivation did not \
         converge within {max_passes} passes over {} model(s) — the model-ref graph likely \
         contains a cycle among maintained `grain: key` composed models (the documented \
         N-model convergence argument assumes an acyclic model-ref graph)",
        models.len()
    );
}

/// One pass of [`derive_clamp_and_locality`]'s per-model derivation, over a
/// caller-supplied `composed_sources` candidate map (an upstream maintained
/// model's own admitted-composed-output `SourceFacts` + declared
/// granularity, as folded by a PRIOR pass — see that function's own doc
/// comment for why this is a fixed-point iteration rather than a single
/// walk). Never mutates its input; the caller drives convergence.
fn derive_clamp_and_locality_pass(
    models: &[ModelFile],
    source_infos: &[SourceInfo],
    composed_sources: &BTreeMap<String, (SourceFacts, Granularity)>,
    workspace_verdicts: &BTreeMap<String, OutputDeltaFacts>,
) -> Result<ClampAndLocality> {
    let model_by_addr: BTreeMap<String, &ModelFile> =
        models.iter().map(|m| (m.canonical_path(), m)).collect();

    // (upstream, downstream) -> widest (before_seconds, after_seconds) seen across
    // every cell that derives a clamp for that pair.
    let mut clamp_seconds: BTreeMap<(String, String), (i64, i64)> = BTreeMap::new();
    // (upstream, downstream) -> the derived write footprint in whole days,
    // widened the same way, downgrading to `None` the moment any
    // contributing cell for that pair carried no derived footprint
    // (`ClampAndLocality::footprint_seconds`'s own doc comment).
    let mut footprint_seconds: BTreeMap<(String, String), Option<(i64, i64)>> = BTreeMap::new();
    // Fold a clamp's derived footprint into `footprint_seconds` for `key`:
    // widen-never-narrow when both sides have derived a footprint, and
    // fail-closed to `None` forever once any contributing cell has none.
    let fold_footprint =
        |footprint_seconds: &mut BTreeMap<(String, String), Option<(i64, i64)>>,
         key: (String, String),
         new: Option<(i64, i64)>| {
            footprint_seconds
                .entry(key)
                .and_modify(|existing| {
                    *existing = match (*existing, new) {
                        (Some((eb, ea)), Some((nb, na))) => Some((eb.max(nb), ea.max(na))),
                        _ => None,
                    };
                })
                .or_insert(new);
        };

    // Per-address key-temporal-locality verdict for every `grain: key`
    // model this workspace derives a plan for — folded from the SAME
    // `MaintenancePlan::key_locality` `smelt explain` reads (Phase A5),
    // never re-derived (`CLAUDE.md` §"Maintenance-plan purity"). Populated
    // below as every model is visited, then consulted by `model_grain`
    // once the full edge list is assembled.
    let mut locality_admitted: BTreeMap<String, bool> = BTreeMap::new();
    let mut key_locality_slice: BTreeMap<
        String,
        smelt_logical::maintenance::locality::LocalitySlice,
    > = BTreeMap::new();
    let mut key_locality_settle_bound: BTreeMap<
        String,
        smelt_logical::maintenance::locality::SettleBound,
    > = BTreeMap::new();

    for model in models {
        let Some(metadata) = model.metadata.as_deref() else {
            continue;
        };
        if metadata.refresh != Some(RefreshStrategy::Incremental) || metadata.grain.is_none() {
            continue;
        }
        let table = model.canonical_path();
        let sql = smelt_parser::strip_frontmatter(&model.content);

        let mut sources: Vec<SourceFacts> = Vec::new();
        let mut model_edges: Vec<smelt_logical::maintenance::derive::ModelEdge> = Vec::new();
        let mut explicitly_mutable: HashSet<String> = HashSet::new();
        let mut seen: BTreeSet<String> = BTreeSet::new();
        // `(bare name, SourceInfo)` pairs for every declared `sources.*` ref
        // this model reads — the same shape `smelt-db::queries::maintenance::
        // build_key_recurrences` consumes (`smelt-db`'s own
        // `derive_model_maintenance_plan_with_edges` call site,
        // `crates/smelt-db/src/lib.rs`) — so route 3's declared
        // `key_recurrence` sub-route admits identically here as it does for
        // `smelt explain` (previously this call site passed `&[]`
        // unconditionally, so a route-3 declared-sub-route composed node —
        // `examples/web_analytics`'s own flagship `silver.events_deduped` —
        // never established locality in the graph layer and its own bare
        // `PartitionGrain::Keyed` classification made
        // `classify_keyed_edges` fail-loud refuse ANY graph containing it,
        // origin or not).
        let mut source_refs: Vec<(String, Option<SourceInfo>)> = Vec::new();
        for r in &model.refs {
            let segs = r.smelt_ref.to_path();
            let bare = bare_name(&segs);
            if !seen.insert(bare.clone()) {
                continue;
            }
            if segs.first().map(|s| s.as_str()) == Some("sources") {
                if let Some(info) = source_infos.iter().find(|s| s.address_segments == segs) {
                    sources.push(smelt_db::queries::maintenance::source_facts(
                        &bare,
                        Some(info),
                        true,
                    ));
                    if info
                        .mutation_profile
                        .as_ref()
                        .is_some_and(|m| m.kind == SourceMutationKind::Mutable)
                    {
                        explicitly_mutable.insert(bare.clone());
                    }
                    source_refs.push((bare.clone(), Some(info.clone())));
                }
                continue;
            }
            let addr = bare.clone();
            if addr == table {
                // Time-unrolled self-edge (`incremental_models.md`
                // §"Time-unrolled self-edges"): admitted iff the SAME proof
                // ordered-backfill execution requires
                // (`windowing.rs`'s `compute_incremental_windows_ordered`
                // call site) says this self-reference is strictly
                // time-backward — shared derivation, so the two verdicts
                // cannot diverge. `Ok` registers a zero-margin-forward,
                // derived-margin-backward self-edge; `Err` keeps today's
                // fail-loud refusal, naming the derivation's own reason.
                let refs: Vec<String> = model
                    .refs
                    .iter()
                    .map(|r| bare_name(&r.smelt_ref.to_path()))
                    .collect();
                let self_partition_col = metadata
                    .timeseries
                    .as_ref()
                    .map(|ts| ts.partition_column.as_str());
                match smelt_logical::analysis::window_independence::self_edge_clamp(
                    &table,
                    &refs,
                    self_partition_col,
                    &sql,
                ) {
                    Ok(before_days) => {
                        // `self_edge_clamp` returns whole days (ceiled
                        // outward) — scale to the graph's own exact-seconds
                        // representation.
                        let before_seconds = before_days * DAY_SECONDS;
                        let entry = clamp_seconds
                            .entry((table.clone(), table.clone()))
                            .or_insert((0, 0));
                        entry.0 = entry.0.max(before_seconds);
                        // Out of this phase's derivation scope (26a is about
                        // the keyed/partition-addressed clamp footprint, not
                        // the self-edge margin) — mirror the read margin
                        // exactly as `reflect` always has, unchanged.
                        fold_footprint(
                            &mut footprint_seconds,
                            (table.clone(), table.clone()),
                            Some((entry.1, entry.0)),
                        );
                    }
                    Err(reason) => {
                        bail!("MaintenanceGraphUnsupportedNode: {reason}");
                    }
                }
                continue;
            }
            if let Some(upstream_model) = model_by_addr.get(&addr) {
                let up_meta = upstream_model.metadata.as_deref();
                let is_maintained =
                    up_meta.map(|m| m.refresh == Some(RefreshStrategy::Incremental)) == Some(true);
                if is_maintained {
                    // A maintained-model upstream is a plan edge of the same
                    // standing as a `sources.*` ref (`incremental_models.md`
                    // §"Upstream model edges"): route it through the SAME
                    // edge-aware derivation `smelt explain` uses
                    // (`derive_model_maintenance_plan_with_edges` →
                    // `append_model_edge_cells`), so the propagation clamp for
                    // this edge equals the creation cell's clamp and an
                    // upstream whose clock cannot be derived is a recorded
                    // refusal (contributing no walkable edge), never a silently
                    // permissive `MutableSnapshot { allow_full_scan: true }`
                    // whole-table synthesis.
                    let clock_col = up_meta
                        .and_then(|m| m.timeseries.as_ref())
                        .map(|ts| ts.partition_column.clone());
                    // Sibling spellings of `clock_col` within the upstream's
                    // own SQL (`ModelEdge::clock_col_aliases`'s doc comment).
                    let clock_col_aliases = clock_col
                        .as_deref()
                        .map(|c| {
                            smelt_logical::analysis::source_bounds::defining_expr_siblings(
                                &upstream_model.content,
                                c,
                            )
                        })
                        .unwrap_or_default();
                    let unique_key = up_meta
                        .and_then(|m| m.unique_key.clone())
                        .unwrap_or_default();
                    // The upstream's own derived output-delta shape
                    // (`ModelEdge::output_shape`'s doc comment): the meet
                    // across whatever per-column-group verdicts
                    // `upstream_output_delta_groups` derives for it — the
                    // SAME per-workspace fold `build_forward_graph`'s own
                    // `type_edge` call reads, never re-derived differently.
                    // `None` when the upstream contributes no groups at all
                    // (unresolvable upstream) rather than an optimistic
                    // guess.
                    let output_shape = upstream_output_delta_groups(
                        &bare,
                        &model_by_addr,
                        source_infos,
                        workspace_verdicts,
                    )
                    .into_iter()
                    .map(|(_, shape)| shape)
                    .reduce(OutputDelta::meet);
                    model_edges.push(smelt_logical::maintenance::derive::ModelEdge {
                        name: bare.clone(),
                        clock_col,
                        clock_col_aliases,
                        unique_key,
                        output_shape,
                    });
                } else {
                    // A `full`-mode or view upstream delivers no incremental
                    // delta (`incremental_models.md` §"Upstream model edges":
                    // "participates in mutation/backfill triggers only") — it
                    // has no creation cell in `smelt explain` either. It is
                    // still a real dependency the backward-resolution graph
                    // must stage, so register it as an unclocked whole-table
                    // edge via the plain source path (the honest widen for an
                    // input with no interval structure).
                    let partition_col = up_meta
                        .and_then(|m| m.timeseries.as_ref())
                        .map(|ts| ts.partition_column.clone());
                    sources.push(SourceFacts {
                        name: bare.clone(),
                        mutation: PlanMutationProfile::MutableSnapshot,
                        partition_col,
                        unique_key: vec![],
                        allow_full_scan: true,
                    });
                }
            }
        }

        // A `grain: key` model's driving source may itself be another
        // maintained model's locality-admitted composed output, not just a
        // declared `sources:` entry — `derive_new_data`'s `inputs.source(source)`
        // lookup (via the `Trigger::NewData` this model's `SourceFacts` list
        // drives) is already agnostic to provenance, so publish every
        // referenced upstream model that cleared the locality gate in a
        // PRIOR pass into the same `SourceFacts` candidate list a declared
        // source populates — mirroring `smelt-db::lib.rs`'s own
        // `ref_model_source_facts`/`model_source_granularities` handling at
        // its `maintenance_plan_report` call site
        // (`docs/plans/20260719-prod-w8-composed-axes-followups.md` Phase 6).
        // A bare `ModelEdge` alone is not enough: `append_model_edge_cells`
        // only ever clamps a **partition**-addressed downstream's creation
        // cell (`output_partition_col` early return), so a `grain: key`
        // downstream needs the composed upstream as an actual `SourceFacts`
        // entry to get a `Trigger::NewData` cell at all. Scoped to `grain:
        // key` models only — a `grain: partition` downstream's pushdown
        // against a composed upstream is already derived through
        // `smelt-logical`'s own model-graph registry, not this path.
        if metadata.grain == Some(ConfigGrain::Key) {
            for edge in &model_edges {
                if let Some((facts, _)) = composed_sources.get(&edge.name) {
                    if !sources.iter().any(|s| s.name == facts.name) {
                        sources.push(facts.clone());
                    }
                }
            }
        }

        // The locality gate's granularity-equality structural precondition
        // (`smelt_logical::maintenance::locality::establish_locality`)
        // needs the driving source's own declared granularity — the same
        // value `smelt-db`'s `check_file_diagnostics` (the `smelt explain`
        // path) computes via `single_clocked_granularity` over every
        // declared source this model references (now including the
        // composed-output `SourceFacts` just pushed above), so a `grain:
        // key` model admits (or refuses) locality identically here and
        // there. The "exactly one clocked candidate, else undecided" rule
        // (`single_clocked_granularity`) is unchanged — this only widens
        // the candidate pool fed into it.
        let driving_source_granularity = if metadata.grain == Some(ConfigGrain::Key) {
            let clocked_granularities: Vec<Granularity> = sources
                .iter()
                .filter_map(|s| {
                    source_infos
                        .iter()
                        .find(|info| bare_name(&info.address_segments) == s.name)
                        .and_then(|info| info.timeseries.as_ref())
                        .map(|ts| ts.granularity)
                        .or_else(|| composed_sources.get(&s.name).map(|(_, g)| *g))
                })
                .collect();
            smelt_logical::maintenance::locality::single_clocked_granularity(clocked_granularities)
        } else {
            None
        };

        let Some(result) = crate::maintenance_availability::derive_resolved_with_edges(
            &sql,
            &table,
            metadata,
            &sources,
            &explicitly_mutable,
            &model_edges,
            driving_source_granularity,
            // Route 3's declared `key_recurrence` fallback (B3,
            // `incremental_shapes.md` §"Key temporal locality" route 3): the
            // SAME `(bare name, key_recurrence)` list `smelt-db`'s own
            // `derive_model_maintenance_plan_with_edges` call site builds
            // via `build_key_recurrences` over the declared `sources.*`
            // refs this model reads, so a route-3 declared-sub-route
            // composed node admits identically here as it does for `smelt
            // explain` — never a separately re-derived admission.
            &smelt_db::queries::maintenance::build_key_recurrences(&source_refs),
            // The propagation graph walk only reads model-edge `NewData`
            // creation cells — a `ColumnAdded` trigger never affects them,
            // so no deployed-schema snapshot is needed here.
            &[],
            // Same rationale as `build_key_recurrences` above: the SAME
            // declared `referential_integrity` facts `smelt-db`'s own
            // `smelt explain` derivation reaches
            // (`build_source_referential_integrity`), so a declared-route
            // `UpstreamMutation` cell's closure is real here too, never a
            // separately re-derived admission.
            &smelt_db::queries::maintenance::build_source_referential_integrity(&source_refs),
            None,
            None,
            // This walk never reads `PlanCell::technique` or
            // `state_downgrade` (only `trigger`, `scans`, `partition_local`,
            // `key_locality` below) — availability resolution mutates
            // neither of the fields this consumer looks at, so full
            // availability is behaviourally identical to a real per-target
            // resolution here. Still routed through the shared seam (never
            // the bare `smelt-db` function) so the structural
            // one-seam-only rule holds without a second, dialect-aware
            // derivation this module has no per-model dialect to build.
            &smelt_logical::maintenance::availability::StateAvailability::all(),
            &source_refs,
        ) else {
            continue;
        };

        // The admitted key-temporal-locality verdict for this model, if
        // it's `grain: key` — `Some(true)` only once `establish_locality`
        // (`smelt_logical::maintenance::locality`) has admitted it (Phase
        // A5's `key_locality` fold). Consulted by `model_grain` below, once
        // every model in the workspace has been visited.
        if metadata.grain == Some(ConfigGrain::Key) {
            locality_admitted.insert(table.clone(), result.plan.key_locality.is_some());
            if let Some(key_locality) = result.plan.key_locality.as_ref() {
                key_locality_slice.insert(table.clone(), key_locality.slice.clone());
                key_locality_settle_bound.insert(table.clone(), key_locality.settle_bound);
            }
        }

        for cell in &result.plan.cells {
            // A `grain: key` model's `UpstreamMutation` cell over an
            // `AppendOnly` source (phase 19, `docs/outcomes/
            // 20260815-definition-delta-migrate`: newly derivable for a
            // fold-driving `AppendOnly` source, not only an
            // explicitly-declared `MutableSnapshot` one) is key-addressed
            // maintenance dispatched by the live-cell resolvers
            // (`resolve_live_column_scoped_cell`/
            // `resolve_live_membership_recompute_cell`), never a
            // forward-propagation graph edge — a bare keyed model
            // participates in propagation only through its `Trigger::
            // NewData` creation cell's established key-locality verdict
            // (handled below, ~L893), exactly like the bare-keyed-creation-
            // cell exclusion this loop already documents. Without this
            // skip, a bare keyed fold (the common case — no partition axis
            // to bound the fold-driving source's own mutation cell against)
            // would register a spurious zero-margin edge via the generic
            // `PartitionLocal::No` fallback below, breaking the "a
            // keyed-grain model never derives an edge" invariant this graph
            // upholds for a model with no time axis. Scoped to `AppendOnly`
            // only — an unclocked, explicitly-declared `MutableSnapshot`
            // ENRICHMENT source's `UpstreamMutation` cell (e.g. a dimension
            // read in a keyed model's own JOIN) already derived and
            // contributed an edge before this phase, unaffected by it, and
            // is exactly what degrades this model's output-delta shape to
            // `General` for the bare-keyed-origin refusal to catch.
            if metadata.grain == Some(ConfigGrain::Key) {
                if let smelt_logical::maintenance::Trigger::UpstreamMutation { source } =
                    &cell.trigger
                {
                    let is_append_only = sources.iter().any(|s| {
                        &s.name == source && s.mutation == PlanMutationProfile::AppendOnly
                    });
                    if is_append_only {
                        continue;
                    }
                }
            }
            for clamp in &cell.scans {
                let e = Edge::from_clamp(&table, clamp);
                let entry = clamp_seconds
                    .entry((clamp.source.clone(), table.clone()))
                    .or_insert((0, 0));
                entry.0 = entry.0.max(e.before_seconds);
                entry.1 = entry.1.max(e.after_seconds);
                fold_footprint(
                    &mut footprint_seconds,
                    (clamp.source.clone(), table.clone()),
                    e.footprint_seconds,
                );
            }
            // A read the derivation could not bound (`PartitionLocal::No`)
            // carries no `ScanClamp` — `cell.scans` stays empty for it
            // (`PlanCell::scans`'s own doc comment: "empty for reads the
            // derivation could not bound — those surface in
            // `partition_local` instead"). Register a zero-margin edge for
            // it anyway so the propagation/resolution graph has a node to
            // walk to at all; the source's own grain (`Unclocked` for an
            // undeclared-timeseries source or model — see `source_grain`/
            // `model_grain` below) is what actually widens every dirty/
            // required interval through it to `PartitionInterval::WHOLE` via
            // `PartitionGrain::align_outward`, never this margin.
            // `incremental_models.md` §"Backward resolution — what must
            // exist": "The required slice of an unclocked source is the
            // whole table."
            if let PartitionLocal::No { source, .. } = &cell.partition_local {
                clamp_seconds
                    .entry((source.clone(), table.clone()))
                    .or_insert((0, 0));
                // No `ScanClamp` exists here at all — out of this phase's
                // scope, unchanged from before it: keep the pre-existing
                // zero-margin exact behaviour rather than introducing a new
                // widen-to-`WHOLE` path this branch never had.
                fold_footprint(
                    &mut footprint_seconds,
                    (source.clone(), table.clone()),
                    Some((0, 0)),
                );
                continue;
            }
            // A locality-admitted composed node's own creation cell
            // (`Grain::Key`'s `FoldDelta` corner, `derive_new_data`) is
            // key-addressed, not partition-addressed: it structurally never
            // carries a `ScanClamp` (`partition_local: PartitionLocal::Yes,
            // scans: vec![]` unconditionally — there is no partition axis
            // for `project_source_link` to bound against). This edge's real
            // key→partition inbound margin (B2, "Key→partition dirt
            // projection through composed nodes") is derived from the SAME
            // admitted `KeyLocality` verdict `locality_admitted` above was
            // folded from — `smelt_logical::maintenance::propagate::
            // locality_margin_seconds` maps the verdict's route (exact for
            // routes 1–2, widened by `r` + margins for route 3) to the day
            // margin this edge carries, so the composed node participates
            // in the graph as a clocked node with a REAL inbound edge
            // instead of the placeholder-exact zero margin the pre-B2 state
            // used (and, before B1, no edge at all). Gated on the admitted
            // locality verdict — never on the mere presence of a
            // `timeseries:` block — so a bare keyed model's own creation
            // cell still contributes no edge at all (unchanged from before
            // this phase; its refusal is `NoAdmissibleTechnique` at
            // plan-derivation time or `MaintenanceGraphUnsupportedNode` if
            // it's otherwise reached as a graph node).
            if cell.scans.is_empty() && locality_admitted.get(&table).copied() == Some(true) {
                if let smelt_logical::maintenance::Trigger::NewData { source } = &cell.trigger {
                    let key_locality = result.plan.key_locality.as_ref().expect(
                        "locality_admitted[table] == Some(true) is set (a few lines above) from \
                         result.plan.key_locality.is_some(), so key_locality must be Some here",
                    );
                    let (before_seconds, after_seconds) =
                        smelt_logical::maintenance::propagate::locality_margin_seconds(
                            &key_locality.slice,
                        );
                    let entry = clamp_seconds
                        .entry((source.clone(), table.clone()))
                        .or_insert((0, 0));
                    entry.0 = entry.0.max(before_seconds);
                    entry.1 = entry.1.max(after_seconds);
                    // A locality-admitted composed node is a CLOCKED node
                    // (`model_grain` returns its declared granularity, not
                    // `Keyed`, once locality is admitted) — `Edge::reflect`
                    // DOES consult this field for this edge. Out of this
                    // phase's derivation scope (26a is about the keyed/
                    // partition-addressed `ScanClamp` footprint, not the
                    // key→partition margin), so mirror the read margin
                    // exactly as `reflect` always has for this edge, never
                    // widening it to `WHOLE`.
                    fold_footprint(
                        &mut footprint_seconds,
                        (source.clone(), table.clone()),
                        Some((entry.1, entry.0)),
                    );
                }
            }
        }
    }

    Ok(ClampAndLocality {
        clamp_seconds,
        footprint_seconds,
        locality_admitted,
        key_locality_slice,
        key_locality_settle_bound,
    })
}

/// Refuse fail-loud when a `--source`/`--landed` delta origin names a
/// **bare** keyed model (`grain: key`, no `timeseries:` declared or one
/// declared but locality not established) whose own derived output-delta
/// shape is `General` or absent — even when the origin has no edge in the
/// assembled graph at all. A bare keyed model whose only downstream reader
/// can't derive a clock for it contributes no walkable edge
/// (`build_forward_graph`'s own "underivable upstream clock is a recorded
/// refusal" behaviour), so without this check an origin naming such a
/// model would otherwise be a **silent no-op** — the delta seeds a dirty
/// entry `propagate` never reflects through any edge, and
/// `plan_since_upstream` prints "nothing to run" and exits 0.
///
/// Narrowed (phase 5, `docs/outcomes/20260809-output-delta-typing/outcome.md`):
/// mirrors [`smelt_logical::maintenance::propagate::classify_keyed_edges`]'s
/// own admission rule — a bare keyed origin whose derived output-delta
/// shape carries at least one non-`General` column group (i.e. an
/// admitted `Addressing::Keyed` component, `type_edge` would derive for
/// its own outbound edges) passes through untouched; the origin case just
/// isn't reachable by `classify_keyed_edges`'s edge-only check, since an
/// edge-less origin is never visited by it. Consults the SAME
/// `locality_admitted` verdict [`build_forward_graph`] derives and the SAME
/// `workspace_output_delta_verdicts` fold, never re-implementing either. A
/// **locality-admitted** composed origin (B1–B3, `incremental_models.md`
/// §"Key temporal locality") passes through untouched regardless — this
/// refusal only ever fires on the bare, `General`-or-absent-shape case.
pub(super) fn refuse_bare_keyed_origins(
    models: &[ModelFile],
    source_infos: &[SourceInfo],
    deltas: &[SourceDelta],
) -> Result<()> {
    let model_by_addr: BTreeMap<String, &ModelFile> =
        models.iter().map(|m| (m.canonical_path(), m)).collect();
    let ClampAndLocality {
        locality_admitted, ..
    } = derive_clamp_and_locality(models, source_infos)?;
    let workspace_verdicts = workspace_output_delta_verdicts(models, source_infos);

    for delta in deltas {
        let Some(model) = model_by_addr.get(&delta.source) else {
            continue;
        };
        let Some(metadata) = model.metadata.as_deref() else {
            continue;
        };
        if metadata.grain != Some(ConfigGrain::Key) {
            continue;
        }
        let admitted = locality_admitted
            .get(&delta.source)
            .copied()
            .unwrap_or(false);
        if admitted {
            continue;
        }
        let has_addressable_shape = workspace_verdicts
            .get(&delta.source.to_ascii_lowercase())
            .is_some_and(|facts| {
                facts
                    .columns
                    .iter()
                    .any(|(_, shape)| !matches!(shape, OutputDelta::General { .. }))
            });
        if has_addressable_shape {
            continue;
        }
        bail!(
            "MaintenanceGraphUnsupportedNode: '{}' is keyed-grain without an admitted time \
             axis: it has no partition axis for interval dirt to propagate over. Declare a \
             timeseries: block and establish key temporal locality \
             (docs/specs/incremental_models.md §\"Key temporal locality\") to admit it as a \
             locality-admitted composed node that participates in propagation like any other \
             clocked node, or ensure its own derived output-delta shape is addressable (keyed \
             dirt-sets propagate for an admitted KeyedUpsert shape — \
             docs/specs/incremental_models.md §\"Keyed dirt-sets and the narrowed refusal\")",
            delta.source
        );
    }
    Ok(())
}
