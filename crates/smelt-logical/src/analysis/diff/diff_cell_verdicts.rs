use super::*;

/// The `(group, trigger)` match key rendered as the report's own
/// `<group>@<trigger>` cell address (`docs/outcomes/20260905-property-diff/
/// phases/03-plan.md` "Cell subject").
fn cell_key(v: &CellVerdict) -> String {
    format!("{}@{}", v.group, v.trigger)
}

/// Diff two cell-verdict lists, matched on `(group, trigger)`
/// (`docs/specs/property_diff.md` §"The diff"). `still_maintained` on an
/// added/removed change is whether the *other* side's cell list is
/// non-empty (whether the model remains maintained at all).
pub(super) fn diff_cell_verdicts(old: &[CellVerdict], new: &[CellVerdict]) -> Vec<ChangeKind> {
    let mut changes = Vec::new();
    let old_by_key: BTreeMap<String, &CellVerdict> = old.iter().map(|v| (cell_key(v), v)).collect();
    let new_by_key: BTreeMap<String, &CellVerdict> = new.iter().map(|v| (cell_key(v), v)).collect();

    for (key, old_v) in &old_by_key {
        match new_by_key.get(key) {
            None => {
                // A removed cell is a downgrade only when its trigger source
                // is still read by another surviving cell
                // (`docs/specs/property_diff.md` §"Direction"
                // "cell_added"/"cell_removed" row) — a cell removed because
                // its source was dropped altogether, or the whole model lost
                // maintenance, is `neutral` (see `ChangeKind::direction`).
                let source_survives = old_v.trigger_source.as_ref().is_some_and(|src| {
                    new.iter()
                        .any(|c| c.trigger_source.as_deref() == Some(src.as_str()))
                });
                changes.push(ChangeKind::CellRemoved {
                    cell: key.clone(),
                    old: Box::new((*old_v).clone()),
                    source_survives,
                })
            }
            Some(new_v) => {
                // `group` and `trigger_source` are identical on both sides —
                // the match key (`group`, `trigger`) already guarantees it —
                // so either side's `CellVerdict` supplies them once here.
                // Carried on the matched-cell `ChangeKind` variants below so
                // a story can name the group/source structurally rather than
                // recovering them by string-searching the cell key's own
                // `{group}@{trigger:?}` join text (the re-parse-our-own-
                // output bug class, `CLAUDE.md` §"Source-derived projection").
                let group = old_v.group.clone();
                let trigger_source = old_v.trigger_source.clone();
                let CellVerdict {
                    group: _,
                    trigger: _,
                    corner: old_corner,
                    technique: old_technique,
                    row_identity: old_ri,
                    contract_point: old_cp,
                    state_downgrade: old_sd,
                    trigger_source: _,
                    partition_local: _,
                    locality_reason: _,
                } = *old_v;
                let CellVerdict {
                    group: _,
                    trigger: _,
                    corner: new_corner,
                    technique: new_technique,
                    row_identity: new_ri,
                    contract_point: new_cp,
                    state_downgrade: new_sd,
                    trigger_source: _,
                    partition_local: _,
                    locality_reason: _,
                } = *new_v;
                if old_technique != new_technique {
                    changes.push(ChangeKind::CellTechnique {
                        cell: key.clone(),
                        group: group.clone(),
                        trigger_source: trigger_source.clone(),
                        old: *old_technique,
                        new: *new_technique,
                    });
                }
                if old_corner != new_corner {
                    changes.push(ChangeKind::CellCorner {
                        cell: key.clone(),
                        group: group.clone(),
                        trigger_source: trigger_source.clone(),
                        old: old_corner.clone(),
                        new: new_corner.clone(),
                    });
                }
                if old_ri != new_ri {
                    changes.push(ChangeKind::CellRowIdentity {
                        cell: key.clone(),
                        old: old_ri.clone(),
                        new: new_ri.clone(),
                    });
                }
                if old_cp != new_cp {
                    changes.push(ChangeKind::ContractPoint {
                        cell: key.clone(),
                        old: old_cp.clone(),
                        new: new_cp.clone(),
                    });
                }
                if old_sd != new_sd {
                    changes.push(ChangeKind::StateDowngrade {
                        cell: key.clone(),
                        group: group.clone(),
                        trigger_source: trigger_source.clone(),
                        old: old_sd.clone(),
                        new: new_sd.clone(),
                    });
                }
            }
        }
    }
    for (key, new_v) in &new_by_key {
        if !old_by_key.contains_key(key) {
            changes.push(ChangeKind::CellAdded {
                cell: key.clone(),
                new: Box::new((*new_v).clone()),
                still_maintained: !old.is_empty(),
            });
        }
    }
    changes
}

/// Diff two refusal sets, matched on `(code, text)`
/// (`docs/specs/property_diff.md` §"The diff" — a `None`-coded refusal
/// matches only another `None`-coded refusal with the same text).
pub(super) fn diff_refusals(old: &[ProfileRefusal], new: &[ProfileRefusal]) -> Vec<ChangeKind> {
    let mut changes = Vec::new();
    for r in old {
        if !new.contains(r) {
            changes.push(ChangeKind::RefusalRemoved(r.clone()));
        }
    }
    for r in new {
        if !old.contains(r) {
            changes.push(ChangeKind::RefusalAdded(r.clone()));
        }
    }
    changes
}

/// Diff two probe sets, matched on `(fact, cell)`
/// (`docs/specs/property_diff.md` §"The diff"). G2 (`docs/outcomes/
/// 20260905-property-diff` fix round 1): a matched pair's `probe` field
/// (the named diagnostic) is destructured explicitly, with NO `..`, so a
/// change to it cannot be silently dropped the way it previously was — a
/// matched probe whose `probe` field changed emitted nothing at all. There
/// is no dedicated dimension for "the probe's diagnostic changed" (the JSON
/// schema names only `probe_added`/`probe_removed`), so it is reported the
/// same way a renamed column is (spec "The diff": "Renames are not
/// detected... is a removal plus an addition").
pub(super) fn diff_probes(old: &[ProfileProbe], new: &[ProfileProbe]) -> Vec<ChangeKind> {
    let mut changes = Vec::new();
    let key = |p: &ProfileProbe| (p.fact.clone(), p.cell.clone());
    let old_by_key: BTreeMap<(String, String), &ProfileProbe> =
        old.iter().map(|p| (key(p), p)).collect();
    let new_by_key: BTreeMap<(String, String), &ProfileProbe> =
        new.iter().map(|p| (key(p), p)).collect();

    for (k, old_p) in &old_by_key {
        match new_by_key.get(k) {
            None => changes.push(ChangeKind::ProbeRemoved((*old_p).clone())),
            Some(new_p) => {
                let ProfileProbe {
                    fact: _,
                    probe: old_probe,
                    cell: _,
                } = *old_p;
                let ProfileProbe {
                    fact: _,
                    probe: new_probe,
                    cell: _,
                } = *new_p;
                if old_probe != new_probe {
                    changes.push(ChangeKind::ProbeRemoved((*old_p).clone()));
                    changes.push(ChangeKind::ProbeAdded((*new_p).clone()));
                }
            }
        }
    }
    for (k, new_p) in &new_by_key {
        if !old_by_key.contains_key(k) {
            changes.push(ChangeKind::ProbeAdded((*new_p).clone()));
        }
    }
    changes
}
