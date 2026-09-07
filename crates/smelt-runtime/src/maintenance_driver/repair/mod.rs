use super::*;
use anyhow::{bail, Result};
use smelt_dialect::SqlDialect;
use smelt_logical::maintenance::choice::ChosenTechnique;
use smelt_logical::maintenance::emit::{widened_scan_predicate, Region};
use smelt_logical::maintenance::{PlanCell, RowIdentity, ScanClamp, Technique};

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
mod execute;
mod resolve_cell;
pub use execute::{
    diff_patch_staged_relation, execute_diff_patch, execute_per_group_recompute,
    repair_affected_keys_select, repair_augmented_model_sql, repair_candidate_select,
    repair_slice_predicate, repair_staged_relation, RepairSidecarRefresh,
};
pub use resolve_cell::resolve_live_per_group_recompute_cell;
