use super::*;
use anyhow::Result;
use smelt_backend::{Backend, BackendError};
use smelt_logical::analysis::fingerprint::Projection as FingerprintProjection;
use smelt_logical::analysis::join_shape::JoinContext;
use smelt_logical::analysis::walk::model_property_vector;
use smelt_logical::maintenance::availability::StateAvailability;
use smelt_logical::maintenance::choice::{
    effective_override, enrichment_restrict_column, resolve_recompute_restriction,
    resolve_region_write_variant, ChoiceRefusal, RecomputeRestriction, RegionWrite,
};
use smelt_logical::maintenance::derive::SourceReferentialIntegrity;
use smelt_logical::maintenance::diff_patch::DeleteLeg;
use smelt_logical::maintenance::emit::{
    emit_count_preservation_probe_from_body, emit_delete_insert,
    emit_delete_insert_delta_restricted, emit_diff_patch, MaintenanceDialect, Region,
    StatementGroup,
};
use smelt_logical::maintenance::{
    RowIdentity, RowPreservation, SkeletonSourceClosure, SourceFacts, Trigger,
};
use std::collections::HashSet;

/// Pure: decide the [`RecomputeRestriction`] verdict and build the
/// resulting [`StatementGroup`] — the single decision-and-emit call site
/// both [`execute_delete_insert_with_delta_restriction`]'s live executor
/// AND the `--dry-run`/`smelt explain` reporting path in
/// `crate::execute::execute_project` route through, so a dry-run's reported
/// statement can never structurally diverge from what a live run with the
/// same inputs would emit (`docs/specs/cli.md` §"`--dry-run` prints the
/// maintenance statements"). A dry-run has no backend to consult, so it
/// always calls this with `observed_delta: None` — [`resolve_recompute_
/// restriction`] then always resolves `Unrestricted`, so a dry-run's
/// reported text is always the ordinary widened scan (the honest choice:
/// a dry-run cannot know whether a live run's delta read would restrict).
///
/// `region_write` is the region family's own change-suppressed conditional
/// variant ([`RegionWrite`], `docs/specs/model_transforms.md` §"Change-
/// suppressed MERGE and the staged-candidate conditional DELETE+INSERT") —
/// consulted only when the delta-restriction arm above does not apply
/// (delta restriction narrows the scan itself, strictly cheaper, so it
/// always wins when both are admitted). A `RegionWrite::Suppressed` verdict
/// realises [`emit_diff_patch`] over the region's own slice predicate,
/// `DeleteLeg::Complete` (a region recompute's candidate covers its own
/// slice by construction — the same grant `resolve_cell_choice` already
/// makes for this corner); `None`/`Unconditional` falls back to today's
/// byte-identical widened scan.
#[allow(clippy::too_many_arguments)]
pub fn build_delete_insert_group_dispatched(
    table: &str,
    partition_col: &str,
    region: &Region,
    body: &str,
    restrict_column: Option<&str>,
    skeleton_source_closure: Option<&SkeletonSourceClosure>,
    observed_delta: Option<&[String]>,
    region_write: Option<&RegionWrite>,
    dialect: MaintenanceDialect,
) -> StatementGroup {
    let restriction = resolve_recompute_restriction(skeleton_source_closure, observed_delta);
    match (restrict_column, restriction) {
        (Some(col), RecomputeRestriction::Restricted { delta_keys }) => {
            emit_delete_insert_delta_restricted(
                table,
                partition_col,
                region,
                body,
                col,
                &delta_keys,
                dialect,
            )
        }
        _ => match region_write {
            Some(RegionWrite::Suppressed {
                key,
                compared_columns,
            }) => {
                // `table` here is already schema-qualified (`emit_delete_insert`'s
                // own convention for this function's `table` parameter) —
                // `diff_patch_staged_relation` assumes a bare table name, so a
                // qualified name is sanitised inline rather than reused
                // verbatim: an embedded `.` would otherwise parse as a second
                // schema qualifier on the staged temp relation's own name.
                let staged_relation = format!("__smelt_diff_patch_{}", table.replace('.', "_"));
                let slice_predicate = region.predicate(Some(table), partition_col);
                emit_diff_patch(
                    table,
                    &staged_relation,
                    key,
                    body,
                    compared_columns,
                    &slice_predicate,
                    &DeleteLeg::Complete,
                    dialect,
                )
            }
            _ => emit_delete_insert(table, partition_col, region, body, dialect),
        },
    }
}

/// Where an exact changed-key delta comes from for
/// [`execute_delete_insert_with_delta_restriction`]'s restriction attempt —
/// the T3 model-edge route (`read_observed_delta_changed_keys`, unchanged
/// from before this variant existed) or the F3/T3-external fingerprint-
/// sidecar route for a `mutable_snapshot` external source
/// (`diff_fingerprint_sidecar_changed_keys`). One executor consumes either
/// shape: the probe dispatch, `resolve_recompute_restriction` call and
/// emitter path stay identical regardless of which variant is passed
/// (`docs/outcomes/20260815-definition-delta-migrate/phases/27e-plan.md`).
#[derive(Debug, Clone, Copy)]
pub enum RestrictionDeltaSource<'a> {
    /// The upstream is a maintained model — read its recorded
    /// `_smelt_observed_delta` row for `[window_start, window_end)`.
    ModelEdge {
        /// The driving model edge's bare address.
        upstream_model: &'a str,
        window_start: &'a str,
        window_end: &'a str,
    },
    /// The upstream is an external `mutable_snapshot` source with no native
    /// change feed — diff the fingerprint sidecar against the source's
    /// current content.
    ExternalSidecar {
        source_address: &'a str,
        source_table: &'a str,
        source_key: &'a [String],
        projection: &'a FingerprintProjection,
        all_source_columns: &'a [String],
        model_sql: &'a str,
        consumer_address: &'a str,
    },
}

/// Execute a model-edge creation-trigger region recompute, restricting it to
/// an exact upstream delta's changed-key set when licensed
/// ([`resolve_recompute_restriction`]'s two-factor admission: P1 skeleton-
/// source closure `Closed` ∧ a non-empty recorded delta). Falls back to the
/// ordinary widened-scan [`emit_delete_insert`] — byte-identical to today's
/// unrestricted region recompute — for an `Open`/absent `skeleton_source_
/// closure`, no `restrict_column` (the cell has no proven row identity to
/// restrict on), an absent delta, or a present-but-empty one.
///
/// Returns the [`StatementGroup`] actually executed, mirroring
/// `execute_column_scoped_write_with_observed_delta`'s shape so a caller
/// (and a test) can assert on exactly what ran.
///
/// `body` and `probe_body` are deliberately two different strings: `body`
/// is what the emitted DELETE/INSERT actually executes (a caller compiling
/// through `SqlCompiler` passes its type-cast-wrapped `CompiledModel::sql`),
/// while `probe_body` is the pre-wrap body the count-preservation probe
/// below reads its enrichment join from (`CompiledModel::body_sql`). Never
/// derive one from the other here — see
/// `docs/plans/20260819-source-derived-projection.md` Phase 5.
///
/// `delta_source` selects where the changed-key set is read from
/// ([`RestrictionDeltaSource`]) — the acquisition step is the only part of
/// this function that varies by source; the probe dispatch and emitter path
/// below are shared unconditionally.
///
/// `ensure_sqls`/`pre_write_sqls` are the same reconciliation-ledger
/// bookkeeping `execute.rs`'s plain DeleteInsert branch attaches
/// (`docs/specs/incremental_models.md` §"The reconciliation ledger";
/// `docs/outcomes/20260904-state-residency/outcome.md`) — empty by default
/// for callers with no ledger reset to attach (e.g. the probe-driven tests
/// in `statement_parity.rs` that call this function directly). When either
/// is non-empty the terminal write routes through
/// [`Backend::execute_write_with_bookkeeping`] so the reset and the write
/// share one backend transaction where the backend can provide one (DuckDB
/// does); when both are empty the call is byte-identical to the pre-phase-3
/// `execute_statement_group` path, so every existing direct caller is
/// unaffected.
#[allow(clippy::too_many_arguments)]
pub async fn execute_delete_insert_with_delta_restriction(
    backend: &dyn Backend,
    schema: &str,
    table: &str,
    partition_col: &str,
    region: &Region,
    body: &str,
    probe_body: &str,
    restrict_column: Option<&str>,
    skeleton_source_closure: Option<&SkeletonSourceClosure>,
    delta_source: RestrictionDeltaSource<'_>,
    region_write: Option<&RegionWrite>,
    dialect: MaintenanceDialect,
    retry: &crate::execute::RetryPolicy<'_>,
    probe_policy: &crate::probes::ProbePolicy,
    ensure_sqls: &[String],
    pre_write_sqls: &[String],
) -> std::result::Result<StatementGroup, BackendError> {
    let full_table = format!("{schema}.{table}");
    let closed = skeleton_source_closure.is_some_and(|c| c.is_closed());
    let mut delta = if restrict_column.is_some() && closed {
        match delta_source {
            RestrictionDeltaSource::ModelEdge {
                upstream_model,
                window_start,
                window_end,
            } => {
                read_observed_delta_changed_keys(
                    backend,
                    schema,
                    upstream_model,
                    window_start,
                    window_end,
                )
                .await?
            }
            RestrictionDeltaSource::ExternalSidecar {
                source_address,
                source_table,
                source_key,
                projection,
                all_source_columns,
                model_sql,
                consumer_address,
            } => Some(
                diff_fingerprint_sidecar_changed_keys(
                    backend,
                    schema,
                    source_address,
                    source_table,
                    source_key,
                    projection,
                    all_source_columns,
                    model_sql,
                    consumer_address,
                )
                .await?,
            ),
        }
    } else {
        None
    };
    // The declared-`referential_integrity` route's row-preservation leg is
    // an *unverified* world-fact until a run actually probes it
    // (`model_properties.md` §"Skeleton-source closure", §"Probe
    // obligation"): a `JoinShape` (`LEFT JOIN`) route needs no runtime
    // check, but a `DeclaredReferentialIntegrity` route must dispatch the
    // count-preservation probe over the touched region *before* trusting
    // the restriction it licenses — never after the write, and never
    // silently skipped.
    let restriction_taken =
        restrict_column.is_some() && delta.as_deref().is_some_and(|d| !d.is_empty());
    if restriction_taken {
        if let Some(SkeletonSourceClosure::Closed {
            row_preservation: RowPreservation::DeclaredReferentialIntegrity { source },
        }) = skeleton_source_closure
        {
            // `emit_count_preservation_probe_from_body` matches the join
            // it finds in `probe_body` against this name, exact-or-last-
            // segment. For a model edge, `source`'s bare address (the
            // closure's own logical derivation) is that name — a
            // maintained model's physical table name has no extra
            // naming-convention prefix over its bare address. For an
            // external source, it is NOT: the compiler's `sources_`
            // naming convention (`<schema>.sources_<address_segments>`)
            // means "raw.users"'s physical table is "sources_raw_users",
            // whose own last dot-segment never matches "raw.users"'s
            // ("users") — so the closure's bare address must be swapped
            // for `delta_source`'s own already-physical `source_table`
            // here, or the probe silently never finds its join and the
            // whole declared-route restriction falls back to the widened
            // scan on every external-source live run
            // (`docs/outcomes/20260815-definition-delta-migrate/phases/
            // 27e-plan.md` — discovered by this phase's own end-to-end
            // test).
            let probe_enrichment_source = match delta_source {
                RestrictionDeltaSource::ExternalSidecar { source_table, .. } => source_table,
                RestrictionDeltaSource::ModelEdge { .. } => source.as_str(),
            };
            let ctx = crate::probes::ProbeContext {
                probe_code: "SourceCountPreservationViolated".to_string(),
                fact: "referential_integrity".to_string(),
                model: table.to_string(),
                cell: format!("{full_table} declared-route delta restriction"),
                remedy: "correct or backfill the dimension's missing key, or drop the \
                         declaration"
                    .to_string(),
            };
            // The count-preservation probe's `driving_count`/`enriched_count`
            // row shape does not match the shared `violation_count`/
            // `sample_keys` contract `dispatch_probe` parses, so this site
            // consults `should_dispatch` directly (the same cadence policy,
            // the same single decision function) rather than reusing
            // `dispatch_probe`'s generic executor.
            match smelt_logical::maintenance::should_dispatch(
                probe_policy.cadence,
                probe_policy.run_ordinal,
            ) {
                smelt_logical::maintenance::ProbeDispatch::Skip(_) => {}
                smelt_logical::maintenance::ProbeDispatch::Dispatch => {
                    match emit_count_preservation_probe_from_body(
                        probe_body,
                        probe_enrichment_source,
                    ) {
                        Some(probe) => {
                            let batches = backend.execute_sql(&probe.sql).await?;
                            let rows = crate::check_runner::batches_to_rows(&batches);
                            let (driving_count, enriched_count) = rows
                                .first()
                                .and_then(|r| {
                                    Some((
                                        r.get("driving_count")?.clone(),
                                        r.get("enriched_count")?.clone(),
                                    ))
                                })
                                .ok_or_else(|| BackendError::ExecutionFailed {
                                    model: table.to_string(),
                                    message: format!(
                                        "count-preservation probe for declared \
                                         referential_integrity on '{source}' returned no \
                                         driving_count/enriched_count row — refusing to trust \
                                         an unchecked declared-route narrowing"
                                    ),
                                })?;
                            let driving_count: i64 = driving_count.parse().unwrap_or(i64::MAX);
                            let enriched_count: i64 = enriched_count.parse().unwrap_or(-1);
                            if enriched_count < driving_count {
                                return Err(BackendError::ExecutionFailed {
                                    model: table.to_string(),
                                    message: format!(
                                        "SourceCountPreservationViolated: '{source}' declares \
                                         referential_integrity, but the enrichment join over the \
                                         touched region ({}..{}) returned \
                                         {enriched_count} row(s) against {driving_count} driving \
                                         row(s) — some driving row's join key has no match in the \
                                         dimension; correct or backfill the dimension's missing \
                                         key, or drop the declaration.{}",
                                        region.start,
                                        region.end,
                                        crate::probes::probe_violation_suffix(&ctx)
                                    ),
                                });
                            }
                        }
                        None => {
                            tracing::warn!(
                                source = %source,
                                table = %table,
                                "declared referential_integrity closure could not build a \
                                 count-preservation probe from this model's own body — dropping \
                                 the delta restriction and falling back to the widened scan"
                            );
                            delta = None;
                        }
                    }
                }
            }
        }
    }
    let group = build_delete_insert_group_dispatched(
        &full_table,
        partition_col,
        region,
        body,
        restrict_column,
        skeleton_source_closure,
        delta.as_deref(),
        region_write,
        dialect,
    );
    if ensure_sqls.is_empty() && pre_write_sqls.is_empty() {
        crate::execute::retry_backend_call(retry, || backend.execute_statement_group(&group))
            .await?;
    } else {
        crate::execute::retry_backend_call(retry, || {
            backend.execute_write_with_bookkeeping(ensure_sqls, pre_write_sqls, &group)
        })
        .await?;
    }
    Ok(group)
}

/// The facts [`build_delete_insert_group_dispatched`]/
/// [`execute_delete_insert_with_delta_restriction`] need to attempt T3 delta
/// restriction for a model-edge-sourced creation cell, resolved by
/// [`resolve_live_delta_restriction_facts`].
#[derive(Debug, Clone)]
pub struct DeltaRestrictionFacts {
    /// The driving model edge's bare address (`Trigger::NewData`'s `source`
    /// name) — the upstream whose observed-delta table is read.
    pub upstream_model: String,
    /// The model's own region row identity, when it resolves to exactly one
    /// column (`RowIdentity::Key(_)` with one element) — this phase's
    /// semi-join restriction is single-column only. `None` for a composite
    /// key or `RowIdentity::WholeRow`, in which case the caller must fall
    /// back to the ordinary widened scan (matching an absent P1 closure).
    pub restrict_column: Option<String>,
    /// The cell's P1 skeleton-source-closure verdict, carried through
    /// unchanged for [`resolve_recompute_restriction`] to consult.
    pub skeleton_source_closure: Option<SkeletonSourceClosure>,
    /// The region family's own change-suppressed conditional-write verdict
    /// ([`RegionWrite`]), resolved from the SAME `Trigger::NewData` cell
    /// this struct already reads — the T3 delta-restriction arm wins when
    /// both are admitted ([`build_delete_insert_group_dispatched`]'s own
    /// match order), so this is consulted only as the fallback dimension.
    pub region_write: RegionWrite,
}

/// Resolve [`DeltaRestrictionFacts`] for a model driven (at least in part)
/// by an upstream **maintained-model** edge (`model_edges`, built by the
/// caller mirroring `crate::propagation::derive_clamp_and_locality`'s own
/// edge extraction — never re-derived here). Routes through the SAME
/// edge-aware derivation `smelt explain`/the propagation graph already
/// consume (`derive_model_maintenance_plan_with_edges` →
/// `append_model_edge_cells`) rather than re-implementing admission
/// (`CLAUDE.md` §"Maintenance-plan purity").
///
/// `model_edges.first()` is this call's driving edge: `append_model_edge_
/// cells` derives ONE shared P1 closure verdict for every edge of a model
/// (see that function's own doc comment — the verdict is a property of the
/// model's own query shape, not of which edge triggered the recompute), so
/// picking any one edge's cell yields the same closure either way; the
/// first edge is simply the one whose observed-delta table the caller then
/// reads. A model with more than one maintained-model upstream restricts
/// only against this first edge's delta in this phase — a later phase may
/// widen this to try every edge in turn.
///
/// Returns `None` when `model_edges` is empty, the plan derives no creation
/// cell for the driving edge (e.g. `Refusal::ReachNotDerivable`), or
/// `metadata`'s resolved grain has no partition axis for a model edge to
/// clamp to — the caller's safe default in every `None` case is the
/// ordinary widened scan.
///
/// The [`RegionWrite`] variant is resolved from the SAME cell via
/// [`resolve_region_write_variant`] — composed from the shared P2/P3 proof
/// (`choice::resolve_write_suppression`/`resolve_write_variant`), never a
/// fresh derivation. A `technique: suppress`/`unconditional` pin on this
/// cell's own trigger address is consulted the same way every other
/// dispatch resolver in this module consults it ([`effective_override`]); an
/// inadmissible hard pin surfaces as a real `Err`, never a silent fallback
/// to the unconditional widened scan.
pub fn resolve_live_delta_restriction_facts(
    sql: &str,
    table: &str,
    metadata: &smelt_core::ModelMetadata,
    sources: &[SourceFacts],
    explicitly_mutable: &HashSet<String>,
    model_edges: &[smelt_logical::maintenance::derive::ModelEdge],
    availability: &StateAvailability,
) -> Result<Option<DeltaRestrictionFacts>, ChoiceRefusal> {
    let Some(driving_edge) = model_edges.first() else {
        return Ok(None);
    };
    let Some(result) = crate::maintenance_availability::derive_resolved_with_edges(
        sql,
        table,
        metadata,
        sources,
        explicitly_mutable,
        model_edges,
        // See `resolve_incremental_strategy`'s analogous call: not (yet)
        // plumbed with a driving-source granularity or declared
        // `key_recurrence` bounds at this call site — this resolver only
        // reads the model-edge creation cell's closure/row-identity facts,
        // which key temporal locality's routes do not gate.
        None,
        &[],
        // This resolver only reads the model-edge `NewData` creation cell —
        // a `ColumnAdded` trigger never affects it, so no deployed-schema
        // snapshot is needed here.
        &[],
        &SourceReferentialIntegrity::new(),
        None,
        None,
        availability,
        &[],
    ) else {
        return Ok(None);
    };
    let Some(cell) = result.plan.cell_for(&Trigger::NewData {
        source: driving_edge.name.clone(),
    }) else {
        return Ok(None);
    };
    let restrict_column = match &cell.row_identity.identity {
        RowIdentity::Key(cols) if cols.len() == 1 => Some(cols[0].clone()),
        _ => None,
    };
    // A model-edge creation cell's `group` is the literal whole-row
    // placeholder (`"{*}"`, `append_model_edge_cells`'s own `PlanCell`
    // construction) — there is no NAMED column group in `result.column_
    // groups` to look it up by, unlike a mutation-sensitive `UpstreamMutation`
    // cell's own derived group. The region family's write covers the whole
    // output row, so its own compared-column set is the model's entire
    // output projection MINUS the proven row-identity's own key columns
    // (comparing a join key via `IS DISTINCT FROM` against itself is
    // vacuous — the diff join already pins it equal for every matched row)
    // — the same `PropertyVector` this call's own comparability read already
    // derives, read once and reused for both.
    let vector = model_property_vector(sql, &JoinContext::new());
    let key_columns: &[String] = match &cell.row_identity.identity {
        RowIdentity::Key(cols) => cols,
        RowIdentity::WholeRow => &[],
    };
    let group_columns: Vec<String> = vector
        .as_ref()
        .map(|v| {
            v.columns
                .iter()
                .filter(|c| !key_columns.iter().any(|k| k.eq_ignore_ascii_case(c)))
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    let comparability = vector.map(|v| v.comparability).unwrap_or_default();
    let cells_cfg: &[smelt_core::config::MaintenanceCellConfig] = metadata
        .maintenance
        .as_ref()
        .map(|m| m.cells.as_slice())
        .unwrap_or(&[]);
    let overrides = effective_override(
        metadata
            .maintenance
            .as_ref()
            .and_then(|m| m.defaults.as_ref()),
        cells_cfg,
        &driving_edge.name,
        &group_columns,
    );
    let region_write = resolve_region_write_variant(
        &group_columns,
        &comparability,
        &cell.row_identity,
        &cell.trigger,
        cell.ledger_catch_up,
        &overrides,
    )?;
    Ok(Some(DeltaRestrictionFacts {
        upstream_model: driving_edge.name.clone(),
        restrict_column,
        skeleton_source_closure: cell.skeleton_source_closure.clone(),
        region_write,
    }))
}

/// The facts [`execute_delete_insert_with_delta_restriction`]'s
/// `RestrictionDeltaSource::ExternalSidecar` arm needs to attempt T3 delta
/// restriction for a region-family cell driven by an external
/// `mutable_snapshot` source with no native change feed, resolved by
/// [`resolve_live_external_delta_restriction_facts`].
#[derive(Debug, Clone)]
pub struct ExternalDeltaRestrictionFacts {
    /// The external source's bare address (`Trigger::UpstreamMutation`'s
    /// `source` name) — the sidecar partition this cell diffs against.
    pub source_name: String,
    /// The model's own region row identity, when it resolves to exactly one
    /// column — mirrors [`DeltaRestrictionFacts::restrict_column`].
    pub restrict_column: Option<String>,
    /// The cell's P1 skeleton-source-closure verdict.
    pub skeleton_source_closure: Option<SkeletonSourceClosure>,
    /// The P4 fingerprint projection the sidecar digests over this source.
    pub projection: FingerprintProjection,
    /// The region family's own change-suppressed conditional-write verdict,
    /// resolved from the SAME `Trigger::UpstreamMutation` cell this struct
    /// already reads.
    pub region_write: RegionWrite,
}

/// Resolve [`ExternalDeltaRestrictionFacts`] for a model driven by an
/// explicitly-mutable **external** source with no maintained-model upstream
/// (`model_edges` empty is the caller's own gate —
/// `docs/outcomes/20260815-definition-delta-migrate/phases/27e-plan.md`).
/// Routes through the SAME edge-aware derivation
/// [`resolve_live_delta_restriction_facts`] uses
/// (`derive_model_maintenance_plan_with_edges` → the `Trigger::
/// UpstreamMutation` cell this source drives), never re-deriving admission
/// (`CLAUDE.md` §"Maintenance-plan purity").
///
/// `explicitly_mutable`'s driving source is picked deterministically
/// (lexicographically smallest — `HashSet` iteration order is unspecified),
/// mirroring `resolve_live_delta_restriction_facts`'s `model_edges.first()`
/// "restrict only against one driving delta in this phase" scoping.
///
/// Returns `None` — logging a `tracing::debug!` `why` — when
/// `supports_fingerprint_sidecar` is `false` for this backend, no
/// explicitly-mutable source exists, the plan derives no `UpstreamMutation`
/// cell for it, the closure is `Open`/absent, the source's declared
/// `unique_key` is composite/undeclared
/// ([`enrichment_restrict_column`] returns `None`), or no P4 fingerprint
/// projection resolved for it — every `None` case is the caller's safe
/// default of falling back to the ordinary widened scan.
#[allow(clippy::too_many_arguments)]
pub fn resolve_live_external_delta_restriction_facts(
    sql: &str,
    table: &str,
    metadata: &smelt_core::ModelMetadata,
    sources: &[SourceFacts],
    explicitly_mutable: &HashSet<String>,
    source_referential_integrity: &SourceReferentialIntegrity,
    supports_fingerprint_sidecar: bool,
    availability: &StateAvailability,
) -> Result<Option<ExternalDeltaRestrictionFacts>, ChoiceRefusal> {
    if !supports_fingerprint_sidecar {
        tracing::debug!(
            table,
            "backend does not declare supports_fingerprint_sidecar — external delta \
             restriction stays out of reach, falling back to the widened scan"
        );
        return Ok(None);
    }
    let mut candidates: Vec<&String> = explicitly_mutable.iter().collect();
    candidates.sort();
    let Some(source_name) = candidates.into_iter().next() else {
        return Ok(None);
    };
    let Some(source_facts) = sources.iter().find(|s| &s.name == source_name) else {
        return Ok(None);
    };
    let Some(result) = crate::maintenance_availability::derive_resolved_with_edges(
        sql,
        table,
        metadata,
        sources,
        explicitly_mutable,
        &[],
        None,
        &[],
        &[],
        source_referential_integrity,
        None,
        None,
        availability,
        &[],
    ) else {
        return Ok(None);
    };
    let trigger = Trigger::UpstreamMutation {
        source: source_name.clone(),
    };
    let Some(cell) = result.plan.cell_for(&trigger) else {
        tracing::debug!(
            table,
            source = %source_name,
            "no UpstreamMutation cell admitted for this source — external delta restriction \
             falls back to the widened scan"
        );
        return Ok(None);
    };
    if !cell
        .skeleton_source_closure
        .as_ref()
        .is_some_and(|c| c.is_closed())
    {
        tracing::debug!(
            table,
            source = %source_name,
            "skeleton-source closure is Open/absent — external delta restriction falls back \
             to the widened scan"
        );
        return Ok(None);
    }
    let Some(restrict_column) = enrichment_restrict_column(&source_facts.unique_key) else {
        tracing::debug!(
            table,
            source = %source_name,
            "source's declared unique_key is composite or undeclared — external delta \
             restriction falls back to the widened scan"
        );
        return Ok(None);
    };
    let Some(projection) = cell.fingerprint_projections.get(source_name).cloned() else {
        tracing::debug!(
            table,
            source = %source_name,
            "no P4 fingerprint projection resolved for this source — external delta \
             restriction falls back to the widened scan"
        );
        return Ok(None);
    };
    let group_columns: Vec<String> = result
        .column_groups
        .iter()
        .find(|g| g.name() == cell.group)
        .map(|g| g.columns.clone())
        .unwrap_or_default();
    let comparability = model_property_vector(sql, &JoinContext::new())
        .map(|v| v.comparability)
        .unwrap_or_default();
    let cells_cfg: &[smelt_core::config::MaintenanceCellConfig] = metadata
        .maintenance
        .as_ref()
        .map(|m| m.cells.as_slice())
        .unwrap_or(&[]);
    let overrides = effective_override(
        metadata
            .maintenance
            .as_ref()
            .and_then(|m| m.defaults.as_ref()),
        cells_cfg,
        source_name,
        &group_columns,
    );
    let region_write = resolve_region_write_variant(
        &group_columns,
        &comparability,
        &cell.row_identity,
        &cell.trigger,
        cell.ledger_catch_up,
        &overrides,
    )?;
    Ok(Some(ExternalDeltaRestrictionFacts {
        source_name: source_name.clone(),
        restrict_column: Some(restrict_column.to_string()),
        skeleton_source_closure: cell.skeleton_source_closure.clone(),
        projection,
        region_write,
    }))
}
