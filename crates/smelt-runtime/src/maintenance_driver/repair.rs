use super::*;
use anyhow::{bail, Result};
use smelt_backend::{maintenance_dialect, Backend, BackendError, ExecutionResult};
use smelt_dialect::SqlDialect;
use smelt_logical::analysis::fingerprint::Projection as FingerprintProjection;
use smelt_logical::analysis::join_shape::JoinContext;
use smelt_logical::analysis::walk::model_property_vector;
use smelt_logical::maintenance::availability::StateAvailability;
use smelt_logical::maintenance::choice::{
    effective_override, resolve_cell_choice, ChosenTechnique,
};
use smelt_logical::maintenance::derive::SourceReferentialIntegrity;
use smelt_logical::maintenance::emit::{emit_per_group_recompute, widened_scan_predicate, Region};
use smelt_logical::maintenance::repair::{discovery_posture, RepairDiscoveryPosture};
use smelt_logical::maintenance::{
    PlanCell, RowIdentity, ScanClamp, SourceFacts, Technique, Trigger,
};
use std::collections::HashSet;
use std::time::Instant;

/// Which write leg a live `Technique::PerGroupRecompute` cell resolves to —
/// the repair family's own targeted `DELETE`+`INSERT`
/// ([`execute_per_group_recompute`]), or a `write: diff_patch` pin over that
/// same cell ([`execute_diff_patch`]). Both read the identical affected-key
/// set, candidate select and key — only the write leg differs
/// (`docs/outcomes/20260809-repair-family/phases/07-plan.md`), so this is a
/// mode carried alongside the resolved cell rather than a second resolver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepairWrite {
    /// `ChosenTechnique::Admitted(Technique::PerGroupRecompute)` — the
    /// repair family's own targeted delete+insert.
    TargetedDeleteInsert,
    /// `ChosenTechnique::DiffPatch { recompute: Technique::PerGroupRecompute,
    /// .. }`, admitted via [`smelt_logical::maintenance::diff_patch::
    /// admit_diff_patch`].
    DiffPatch {
        compared_columns: Vec<String>,
        delete_leg: smelt_logical::maintenance::diff_patch::DeleteLeg,
    },
}

/// How a live `Technique::PerGroupRecompute` cell discovers its affected-key
/// relation (P9, `docs/specs/incremental_models.md` §"The repair family" —
/// "Obligation 7 over a `mutable_snapshot` source"): the append-only clamped
/// rescan, or — for a [`smelt_logical::maintenance::MutationProfile::MutableSnapshot`] source with no
/// native change feed — the group-grain fingerprint sidecar diff, the only
/// route that can witness a group whose entire window contribution departed
/// the source (a vanished group leaves no row for a clamped scan to select,
/// but the sidecar keeps a stored comparandum for it).
#[derive(Debug, Clone)]
pub enum RepairDiscovery {
    /// The append-only widened-scan `SELECT DISTINCT`
    /// ([`repair_affected_keys_select`]).
    ClampedScan,
    /// The group-grain sidecar diff ([`diff_repair_group_sidecar_changed_keys`]).
    /// `digest_columns` is the P4 fingerprint projection's column set for
    /// this source, read straight off the cell's own derived
    /// `fingerprint_projections` — never re-derived.
    SidecarDiff { digest_columns: Vec<String> },
}

/// A live repair cell as [`resolve_live_per_group_recompute_cell`] returns
/// it: the trigger's source name, the cell itself, its proven
/// `RowIdentity::Key`, the cell's own derived bounded read slice, the
/// resolved write leg ([`RepairWrite`]), and how its affected-key relation
/// is discovered ([`RepairDiscovery`]).
pub type LiveRepairCell = (
    String,
    PlanCell,
    Vec<String>,
    ScanClamp,
    RepairWrite,
    RepairDiscovery,
);

/// Resolve a `Technique::PerGroupRecompute` cell's already-chosen
/// [`ChosenTechnique`] (`resolve_cell_choice`'s output — the override ladder
/// has already run) into the [`RepairWrite`] this lowering executes, or
/// `Ok(None)` when `chosen` is not this cell's live technique at all
/// (`RegionRecompute`, or `Admitted` of a technique other than
/// `PerGroupRecompute` — never reachable in practice since
/// [`resolve_live_per_group_recompute_cell`] only ever calls this with a
/// cell whose own `technique` is `PerGroupRecompute`, but a defensive arm
/// nonetheless).
///
/// Pure and independently unit-testable — split out of
/// `resolve_live_per_group_recompute_cell`'s loop body so the fail-loud
/// `DiffPatch { recompute: <not PerGroupRecompute> }` arm ("a `diff_patch`
/// pin over the region `DeleteInsert` default fails loud by name rather
/// than falling through to the default write",
/// `docs/outcomes/20260809-repair-family/phases/07-plan.md`) is exercisable
/// without needing a full plan derivation to reach it.
pub fn resolve_repair_write(
    chosen: &ChosenTechnique,
    group_columns: &[String],
    comparability: &[smelt_logical::analysis::walk::ColumnComparability],
    row_identity: &smelt_logical::maintenance::RowIdentityVerdict,
    group_label: &str,
) -> Result<Option<RepairWrite>> {
    match chosen {
        ChosenTechnique::Admitted(Technique::PerGroupRecompute) => {
            Ok(Some(RepairWrite::TargetedDeleteInsert))
        }
        ChosenTechnique::DiffPatch {
            recompute: Technique::PerGroupRecompute,
            delete_leg,
        } => {
            let slice_complete = match delete_leg {
                smelt_logical::maintenance::diff_patch::DeleteLeg::Complete => Ok(()),
                smelt_logical::maintenance::diff_patch::DeleteLeg::Omitted { why } => {
                    Err(why.clone())
                }
            };
            let admitted = smelt_logical::maintenance::diff_patch::admit_diff_patch(
                group_columns,
                comparability,
                row_identity,
                slice_complete,
            )
            .map_err(|refusal| {
                anyhow::anyhow!(
                    "MaintenanceDiffPatchRefused: a `write: diff_patch` pin over a \
                     Technique::PerGroupRecompute cell for group '{group_label}' could not be \
                     admitted: {refusal:?}"
                )
            })?;
            Ok(Some(RepairWrite::DiffPatch {
                compared_columns: admitted.compared_columns,
                delete_leg: admitted.delete_leg,
            }))
        }
        ChosenTechnique::DiffPatch { recompute, .. } => {
            bail!(
                "MaintenanceDiffPatchUnroutable: a `write: diff_patch` pin over group \
                 '{group_label}' resolved over technique {recompute:?} — only \
                 Technique::PerGroupRecompute has a diff_patch lowering today; the region \
                 DeleteInsert default fails loud rather than silently falling through to the \
                 default write",
            );
        }
        ChosenTechnique::RegionRecompute | ChosenTechnique::Admitted(_) => Ok(None),
    }
}

/// The proven group key of a `Technique::PerGroupRecompute` cell — the
/// repair family's fail-loud identity check
/// (`docs/specs/incremental_models.md` §"The repair family").
///
/// `smelt_logical::maintenance::repair::derive_repair_cell` only ever builds
/// a `RowIdentity::Key`, so a `WholeRow` (or empty-key) identity on a
/// per-group-recompute cell is an internal inconsistency, never a shape this
/// lowering may quietly widen: a repair with no group key to restrict its
/// `DELETE`/`INSERT` to would rewrite every stored row. Refuse by name
/// instead (root `CLAUDE.md` §"Fail-loud discipline"), never a skip.
pub fn repair_cell_key(cell: &PlanCell) -> Result<Vec<String>> {
    let RowIdentity::Key(key) = &cell.row_identity.identity else {
        bail!(
            "MaintenanceRepairIdentityUnproven: a Technique::PerGroupRecompute cell for group \
             '{}' carries RowIdentity::WholeRow — the repair family recomputes whole key \
             groups and has no meaning without a proven group key; widening it to every stored \
             row is never the fallback",
            cell.group
        );
    };
    if key.is_empty() {
        bail!(
            "MaintenanceRepairIdentityUnproven: a Technique::PerGroupRecompute cell for group \
             '{}' carries an empty RowIdentity::Key — the repair family recomputes whole key \
             groups and has no meaning without a proven group key",
            cell.group
        );
    }
    Ok(key.clone())
}

/// Find the first source whose `Trigger::NewData` cell resolves live to
/// `Technique::PerGroupRecompute` — the repair family's counterpart of
/// [`resolve_live_column_scoped_cell`]/[`resolve_live_membership_recompute_cell`]
/// (`docs/specs/incremental_models.md` §"The repair family").
///
/// Unlike those two, this resolver scans the model's **own driving/fold**
/// trigger, not an enrichment dimension's mutation trigger:
/// `derive_new_data`'s key-grain branch narrows a faithful-fold
/// source-posture refusal into a repair cell attached to that same
/// `Trigger::NewData { source }`
/// (`smelt_logical::maintenance::derive::derive_new_data`). The repair cell
/// is therefore an **alternative to** the `KeyedFold` cell for that trigger,
/// not a technique dispatched alongside one — which is why the keyed run
/// loop routes it *instead of* the cumulative fold rather than after it.
/// `Trigger::UpstreamMutation` cells are scanned too, so a future derivation
/// that admits repair for a mutation trigger needs no second resolver.
///
/// Returns the source name, the cell, its proven `RowIdentity::Key`
/// ([`repair_cell_key`] — a `WholeRow` identity is a fail-loud `bail!`,
/// never a skip) and the cell's own derived [`ScanClamp`] (the bounded
/// per-group read slice `repair::admit_per_group_recompute` proved as
/// obligation 4), and how its affected-key relation is discovered
/// ([`RepairDiscovery`]). The plan is derived exactly once here
/// (maintenance-plan purity, root `CLAUDE.md`); nothing downstream
/// re-derives admission.
///
/// `supports_fingerprint_sidecar` gates [`RepairDiscovery::SidecarDiff`]: a
/// [`smelt_logical::maintenance::MutationProfile::MutableSnapshot`] source routes to the group-grain
/// sidecar diff, which needs a target declaring the capability (matching the
/// per-row sidecar's own posture, `diff_fingerprint_sidecar_changed_keys`) —
/// a target that does not declare it fails loud here, before any backend
/// call, rather than silently falling back to the unsound current-source
/// scan. `dialect` still supplies `.name()` for the refusal message.
#[allow(clippy::too_many_arguments)]
pub fn resolve_live_per_group_recompute_cell(
    sql: &str,
    table: &str,
    metadata: &smelt_core::ModelMetadata,
    sources: &[SourceFacts],
    explicitly_mutable: &HashSet<String>,
    technique_overrides: &[crate::types::CellTechniqueOverride],
    dialect: SqlDialect,
    supports_fingerprint_sidecar: bool,
    availability: &StateAvailability,
) -> Result<Option<LiveRepairCell>> {
    let Some(result) = crate::maintenance_availability::derive_resolved(
        sql,
        table,
        metadata,
        sources,
        explicitly_mutable,
        // Same `None`/`&[]` posture as the sibling resolvers above: the
        // repair cell's own admission (affected-key discovery + bounded
        // slice) reads neither the driving source's declared granularity,
        // nor declared key recurrences, nor a deployed-schema snapshot.
        None,
        &[],
        &[],
        &SourceReferentialIntegrity::new(),
        None,
        None,
        availability,
        &[],
    ) else {
        return Ok(None);
    };
    let cells_cfg: &[smelt_core::config::MaintenanceCellConfig] = metadata
        .maintenance
        .as_ref()
        .map(|m| m.cells.as_slice())
        .unwrap_or(&[]);
    let request_cells: Vec<smelt_core::config::MaintenanceCellConfig> = technique_overrides
        .iter()
        .map(|o| smelt_core::config::MaintenanceCellConfig {
            columns: o.columns.clone(),
            on: o.on.clone(),
            prefer: None,
            technique: Some(o.technique),
            write: None,
        })
        .collect();
    let combined_cells: Vec<smelt_core::config::MaintenanceCellConfig> = request_cells
        .iter()
        .cloned()
        .chain(cells_cfg.iter().cloned())
        .collect();

    for facts in sources {
        for trigger in [
            Trigger::NewData {
                source: facts.name.clone(),
            },
            Trigger::UpstreamMutation {
                source: facts.name.clone(),
            },
        ] {
            let sibling_cells: Vec<PlanCell> = result.plan.cells_for(&trigger).cloned().collect();
            if sibling_cells.is_empty() {
                continue;
            }
            let sibling_group_columns: Vec<Vec<String>> = sibling_cells
                .iter()
                .map(|c| {
                    result
                        .column_groups
                        .iter()
                        .find(|g| g.name() == c.group)
                        .map(|g| g.columns.clone())
                        .unwrap_or_default()
                })
                .collect();
            // Same dangling-pin fail-loud gate the sibling resolvers apply
            // (`docs/plans/20260808-membership-sensitivity.md` Phase 3).
            if let Some(dangling) = smelt_logical::maintenance::choice::unaddressed_technique_pin(
                &combined_cells,
                &facts.name,
                &sibling_group_columns,
            ) {
                bail!(
                    "MaintenanceUnboundedFootprint: cells[on: {}].technique pin (columns: \
                     {:?}) does not address any of this trigger's own derived column groups \
                     ({:?}) — a hard technique pin must name columns belonging to exactly one \
                     of the trigger's admitted cells, never columns absent from every one of \
                     them",
                    facts.name,
                    dangling.columns,
                    sibling_group_columns,
                );
            }
            for (cell, group_columns) in sibling_cells.iter().zip(sibling_group_columns.iter()) {
                if cell.technique != Technique::PerGroupRecompute {
                    continue;
                }
                // Fail-loud BEFORE the choice ladder: an unprovable group
                // key is an internal inconsistency, not an override
                // outcome.
                let key = repair_cell_key(cell)?;
                let write_pin = smelt_db::queries::maintenance::matching_write_pin(
                    cell,
                    &result.column_groups,
                    cells_cfg,
                )
                .and_then(|pin_name| smelt_logical::maintenance::lookup_write_pattern(&pin_name));
                let overrides = effective_override(
                    metadata
                        .maintenance
                        .as_ref()
                        .and_then(|m| m.defaults.as_ref()),
                    &combined_cells,
                    &facts.name,
                    group_columns,
                );
                let chosen = resolve_cell_choice(
                    Some(cell),
                    &trigger,
                    &overrides,
                    write_pin,
                    // `{recompute, PerGroupRecompute}` never contains
                    // `ColumnScopedMerge`, so the backend's MERGE capability
                    // is irrelevant to this resolver's resolvable set — the
                    // same `false` the membership resolver passes.
                    false,
                )
                .map_err(|refusal| anyhow::anyhow!(refusal.to_string()))?;
                // A `technique: recompute` pin (→ `RegionRecompute`) is
                // simply not THIS live cell: skip the source and let the
                // caller's own default apply, exactly as the sibling
                // resolvers do for a choice they have no lowering for. A
                // `write: diff_patch` pin routes here too, but only when its
                // underlying recompute is `PerGroupRecompute` — the sole
                // recompute `resolve_cell_choice` ever grants
                // `DeleteLeg::Complete` (the region `DeleteInsert` default
                // has no lowering and fails loud by name rather than
                // silently falling through to the default write);
                // [`resolve_repair_write`] carries the full decision table.
                let comparability = model_property_vector(sql, &JoinContext::new())
                    .map(|v| v.comparability)
                    .unwrap_or_default();
                let Some(write) = resolve_repair_write(
                    &chosen,
                    group_columns,
                    &comparability,
                    &cell.row_identity,
                    &cell.group,
                )?
                else {
                    continue;
                };
                // The cell's own derived slice — obligation 4's bounded
                // per-group read footprint, matched to the trigger's own
                // source (never another source's clamp).
                let Some(slice) = cell
                    .scans
                    .iter()
                    .find(|c| c.source == facts.name)
                    .or_else(|| cell.scans.first())
                else {
                    bail!(
                        "MaintenanceRepairSliceMissing: a Technique::PerGroupRecompute cell for \
                         group '{}' on source '{}' carries no derived ScanClamp — the bounded \
                         per-group read slice is admission obligation 4 and is never assumed",
                        cell.group,
                        facts.name,
                    );
                };
                // P9 (`docs/specs/incremental_models.md` §"The repair
                // family" — "Obligation 7 over a `mutable_snapshot`
                // source"): a source with no native change feed and no
                // tombstone/change history needs the group-grain sidecar
                // diff to witness a wholly-deleted group; every other
                // posture keeps the ordinary clamped current-source scan. A
                // `ChangeFeed` source has no discovery posture at all — the
                // repair family is refused for it upstream at derivation
                // time (`derive::derive_new_data`), so a live cell here
                // should never carry one; a `None` posture bails loud
                // rather than silently defaulting to a scan that may drop
                // rows.
                let Some(posture) = discovery_posture(facts.mutation) else {
                    bail!(
                        "MaintenanceRepairDiscoveryPostureMissing: a Technique::\
                         PerGroupRecompute cell for group '{}' resolved a change_feed source \
                         '{}' — the repair family has no fingerprint-sidecar discovery for a \
                         change feed and should never have admitted this cell",
                        cell.group,
                        facts.name,
                    );
                };
                let discovery = if posture == RepairDiscoveryPosture::SidecarDiff {
                    if !supports_fingerprint_sidecar {
                        return Err(BackendError::unsupported(
                            dialect.name(),
                            "group-grain fingerprint-sidecar affected-key discovery for a \
                             mutable_snapshot repair source (P9)",
                        )
                        .into());
                    }
                    let digest_columns: Vec<String> =
                        match cell.fingerprint_projections.get(&facts.name) {
                            Some(FingerprintProjection::Columns(cols)) => {
                                cols.iter().cloned().collect()
                            }
                            _ => Vec::new(),
                        };
                    if digest_columns.is_empty() {
                        bail!(
                            "MaintenanceRepairDigestColumnsMissing: a Technique::PerGroupRecompute \
                             cell for group '{}' resolved a MutableSnapshot delta posture on \
                             source '{}' with no P4 fingerprint projection columns — the \
                             group-grain sidecar digest has nothing to hash",
                            cell.group,
                            facts.name,
                        );
                    }
                    RepairDiscovery::SidecarDiff { digest_columns }
                } else {
                    RepairDiscovery::ClampedScan
                };
                return Ok(Some((
                    facts.name.clone(),
                    cell.clone(),
                    key,
                    slice.clone(),
                    write,
                    discovery,
                )));
            }
        }
    }
    Ok(None)
}

/// The affected-key relation a repair reads: the distinct group keys present
/// in the mutated source over the cell's own bounded slice, projected as a
/// single canonical `delta_key` column — the same expression
/// ([`smelt_logical::maintenance::emit::key_expr_for_columns`])
/// [`emit_fingerprint_sidecar_diff`]/[`emit_repair_group_sidecar_diff`]
/// project, so the append-only clamped-scan path and the `mutable_snapshot`
/// group-grain sidecar-diff path ([`diff_repair_group_sidecar_changed_keys`] +
/// [`repair_keys_literal_select`])
/// yield the SAME one-column relation shape — [`emit_per_group_recompute`]
/// joins against it by key EXPRESSION, never by raw key columns, because a
/// deleted group's typed column values are unrecoverable by construction.
///
/// Pure string builder, per this module's "callers resolve strings, emitters
/// assemble" contract — [`emit_per_group_recompute`] consumes this as opaque
/// `SELECT` text. The clamp is pushed into **this** read (and only this
/// one): the widened source band is what bounds how many groups a repair
/// touches, while the recompute of each touched group must stay unbounded
/// so the group is recomputed whole (see
/// [`repair_candidate_select`]).
///
/// `clamp: None` yields the unpredicated read. That branch is only reachable
/// where admission already proved the slice by another route —
/// `repair::admit_per_group_recompute` refuses `RepairSliceUnbounded` rather
/// than admitting a clamp-less cell, so this is not a silent widening path.
pub fn repair_affected_keys_select(
    source_table: &str,
    key: &[String],
    clamp: Option<&ScanClamp>,
    region: &Region,
) -> String {
    let key_expr = smelt_logical::maintenance::emit::key_expr_for_columns(key);
    match clamp {
        Some(clamp) => format!(
            "SELECT DISTINCT {key_expr} AS delta_key FROM {source_table} WHERE {}",
            widened_scan_predicate(clamp, region)
        ),
        None => format!("SELECT DISTINCT {key_expr} AS delta_key FROM {source_table}"),
    }
}

/// The candidate relation a repair stages: the model's **full** (unwindowed)
/// recompiled SQL, semi-joined to `affected_keys_select`'s single-column
/// `delta_key` relation.
///
/// Full, not windowed, because the repair family's promise is that an
/// affected group's stored value equals a full refresh of that group — a
/// non-invertible combiner (`MAX`) over a retracted contribution cannot be
/// fixed from a window's rows alone. The semi-join is what keeps the
/// recompute *bounded*: only the groups the affected-keys read named are
/// recomputed. `EXISTS` rather than a row-value `IN`, so a composite key
/// lowers identically across dialects. The join compares
/// [`repair_affected_keys_select`]'s own canonical key expression over the
/// candidate's key columns against `delta_key`, mirroring
/// [`repair_slice_predicate`]'s and `emit_per_group_recompute`'s identical
/// shape.
/// Widen `clean_sql` (the model's own raw, pre-compile SELECT) with one
/// `, <per_partition_expr> AS <name>` per `state_columns` — a named wrapper
/// over [`smelt_logical::maintenance::emit::state_augmented_projection`]
/// with the repair path's own error text, so the widening is independently
/// unit-testable rather than inlined at each call site
/// (`docs/outcomes/20260809-repair-family/phases/10-plan.md`). Mirrors
/// `smelt-runtime::cumulative::execute_windowed_keyed`/
/// `execute_snapshot_reconcile`'s own use of the same emitter: the fold's
/// create/merge path already carries a decomposed combiner's hidden state
/// columns in the physical table, so a repair's own candidate/insert must
/// supply them too, or the `INSERT`'s implicit column list mismatches the
/// table. `state_columns.is_empty()` returns `clean_sql` unchanged.
pub fn repair_augmented_model_sql(
    clean_sql: &str,
    state_columns: &[smelt_logical::analysis::decomposed_state::StateColumn],
) -> Result<String> {
    smelt_logical::maintenance::emit::state_augmented_projection(clean_sql, state_columns).map_err(
        |_| {
            anyhow::anyhow!(
                "Failed to append decomposed-state columns to a repair candidate: the model's \
                 SELECT could not be parsed"
            )
        },
    )
}

pub fn repair_candidate_select(
    full_model_sql: &str,
    key: &[String],
    affected_keys_select: &str,
) -> String {
    let candidate_key_columns: Vec<String> = key
        .iter()
        .map(|k| format!("__smelt_repair_candidate.{k}"))
        .collect();
    let candidate_key_expr =
        smelt_logical::maintenance::emit::key_expr_for_columns(&candidate_key_columns);
    format!(
        "SELECT __smelt_repair_candidate.* FROM ({full_model_sql}) AS __smelt_repair_candidate \
         WHERE EXISTS (SELECT 1 FROM ({affected_keys_select}) AS __smelt_repair_keys WHERE \
         {candidate_key_expr} = __smelt_repair_keys.delta_key)"
    )
}

/// Inputs to refresh the group-grain fingerprint sidecar transactionally
/// with a repair write (P9, `docs/outcomes/20260809-repair-family/phases/
/// 09-plan.md` task 6) — passed to [`execute_per_group_recompute`]/
/// [`execute_diff_patch`] only when the live cell's discovery is
/// [`RepairDiscovery::SidecarDiff`]; `None` for the ordinary clamped-scan
/// path, which has no sidecar partition to refresh.
pub struct RepairSidecarRefresh<'a> {
    pub schema: &'a str,
    pub source_address: &'a str,
    pub source_table: &'a str,
    pub group_key: &'a [String],
    pub digest_columns: &'a [String],
    pub model_sql: &'a str,
    pub consumer_address: &'a str,
}

/// Execute a live `Technique::PerGroupRecompute` cell
/// ([`resolve_live_per_group_recompute_cell`]) via the repair family's
/// targeted `DELETE`+`INSERT` over the affected-key relation
/// ([`emit_per_group_recompute`]) — the same emitter → `retry_backend_call`
/// → [`Backend::execute_statement_group`] shape
/// [`execute_staged_membership_recompute`] uses, so the executed text is
/// exactly the single owner's output (`docs/specs/incremental_models.md`
/// §"Statement emission (single owner)").
///
/// `sidecar_refresh: Some(..)` (a [`RepairDiscovery::SidecarDiff`] cell)
/// routes the SAME emitted [`StatementGroup`] through
/// [`refresh_repair_group_sidecar`] instead of a bare
/// [`Backend::execute_statement_group`] call — the group-grain sidecar
/// partition refreshes in the SAME backend transaction as this write
/// (mirroring [`refresh_fingerprint_sidecar`]'s own transactional shape),
/// so a failed write leaves the sidecar untouched rather than
/// half-committed.
#[allow(clippy::too_many_arguments)]
pub async fn execute_per_group_recompute(
    backend: &dyn Backend,
    schema: &str,
    table: &str,
    key: &[String],
    affected_keys_select: &str,
    candidate_select: &str,
    retry: &crate::execute::RetryPolicy<'_>,
    sidecar_refresh: Option<&RepairSidecarRefresh<'_>>,
) -> Result<ExecutionResult> {
    let start = Instant::now();
    let full_table = format!("{schema}.{table}");
    let dialect = maintenance_dialect(backend.dialect());
    let staged_relation = repair_staged_relation(table);
    let group = emit_per_group_recompute(
        &full_table,
        &staged_relation,
        key,
        affected_keys_select,
        candidate_select,
        dialect,
    );
    match sidecar_refresh {
        None => {
            crate::execute::retry_backend_call(retry, || backend.execute_statement_group(&group))
                .await
                .map_err(|e| {
                    anyhow::anyhow!("per-group recompute failed for '{full_table}': {e}")
                })?;
        }
        Some(refresh) => {
            crate::execute::retry_backend_call(retry, || {
                refresh_repair_group_sidecar(
                    backend,
                    refresh.schema,
                    refresh.source_address,
                    refresh.source_table,
                    refresh.group_key,
                    refresh.digest_columns,
                    refresh.model_sql,
                    refresh.consumer_address,
                    &group,
                )
            })
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "per-group recompute (with group-grain sidecar refresh) failed for \
                     '{full_table}': {e}"
                )
            })?;
        }
    }
    let row_count = backend.get_row_count(schema, table).await.unwrap_or(0);
    Ok(ExecutionResult {
        model_name: table.to_string(),
        duration: start.elapsed(),
        row_count,
        preview: None,
    })
}

/// The staged temp relation name a repair uses for `table` — one derivation,
/// so a parity test (and the technique preview) can name the same relation
/// the live run does without guessing.
pub fn repair_staged_relation(table: &str) -> String {
    format!("__smelt_repair_{table}")
}

/// The staged temp relation name a `diff_patch` write over a repair cell
/// uses for `table` — a distinct prefix from [`repair_staged_relation`] so a
/// parity test can name each group's own relation without ambiguity.
pub fn diff_patch_staged_relation(table: &str) -> String {
    format!("__smelt_diff_patch_{table}")
}

/// The `diff_patch` slice restriction for a repair cell: the candidate's own
/// slice is the affected-key set, not a partition region
/// ([`emit_diff_patch`]'s doc comment) — an `EXISTS` over the affected-keys
/// read's single-column `delta_key` relation, `table`-qualified on every key
/// column (via the same canonical key expression
/// [`repair_affected_keys_select`]/[`repair_candidate_select`] use) so it
/// composes unambiguously into both the update-leg and delete-leg `DELETE`s
/// `emit_diff_patch` builds.
pub fn repair_slice_predicate(table: &str, key: &[String], affected_keys_select: &str) -> String {
    let table_key_columns: Vec<String> = key.iter().map(|k| format!("{table}.{k}")).collect();
    let table_key_expr = smelt_logical::maintenance::emit::key_expr_for_columns(&table_key_columns);
    format!(
        "EXISTS (SELECT 1 FROM ({affected_keys_select}) AS __smelt_repair_keys WHERE \
         {table_key_expr} = __smelt_repair_keys.delta_key)"
    )
}

/// Execute a `write: diff_patch` pin over a live `Technique::PerGroupRecompute`
/// cell ([`resolve_live_per_group_recompute_cell`]) via [`emit_diff_patch`] —
/// same emitter → `retry_backend_call` → [`Backend::execute_statement_group`]
/// shape [`execute_per_group_recompute`] uses, so the executed text is
/// exactly the single owner's output (`docs/specs/incremental_models.md`
/// §"Statement emission (single owner)"). `sidecar_refresh` carries the same
/// meaning as [`execute_per_group_recompute`]'s own parameter.
#[allow(clippy::too_many_arguments)]
pub async fn execute_diff_patch(
    backend: &dyn Backend,
    schema: &str,
    table: &str,
    key: &[String],
    candidate_select: &str,
    compared_columns: &[String],
    slice_predicate: &str,
    delete_leg: &smelt_logical::maintenance::diff_patch::DeleteLeg,
    retry: &crate::execute::RetryPolicy<'_>,
    sidecar_refresh: Option<&RepairSidecarRefresh<'_>>,
) -> Result<ExecutionResult> {
    let start = Instant::now();
    let full_table = format!("{schema}.{table}");
    let dialect = maintenance_dialect(backend.dialect());
    let staged_relation = diff_patch_staged_relation(table);
    let group = smelt_logical::maintenance::emit::emit_diff_patch(
        &full_table,
        &staged_relation,
        key,
        candidate_select,
        compared_columns,
        slice_predicate,
        delete_leg,
        dialect,
    );
    match sidecar_refresh {
        None => {
            crate::execute::retry_backend_call(retry, || backend.execute_statement_group(&group))
                .await
                .map_err(|e| anyhow::anyhow!("diff_patch write failed for '{full_table}': {e}"))?;
        }
        Some(refresh) => {
            crate::execute::retry_backend_call(retry, || {
                refresh_repair_group_sidecar(
                    backend,
                    refresh.schema,
                    refresh.source_address,
                    refresh.source_table,
                    refresh.group_key,
                    refresh.digest_columns,
                    refresh.model_sql,
                    refresh.consumer_address,
                    &group,
                )
            })
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "diff_patch write (with group-grain sidecar refresh) failed for \
                     '{full_table}': {e}"
                )
            })?;
        }
    }
    let row_count = backend.get_row_count(schema, table).await.unwrap_or(0);
    Ok(ExecutionResult {
        model_name: table.to_string(),
        duration: start.elapsed(),
        row_count,
        preview: None,
    })
}
