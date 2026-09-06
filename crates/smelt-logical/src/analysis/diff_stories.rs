//! Stories (`docs/specs/property_diff.md` §"Stories"): the short,
//! severity-ranked list of sentences a reviewer reads first, folded from a
//! shifted model's [`Change`] list by one pure function. `narrate` has no
//! SQL knowledge — it reads [`ChangeKind`] values and quotes reason texts
//! that the property derivation already produced
//! (`docs/specs/property_diff.md` §Constraints item 10, "Narration single
//! ownership"). Every change belongs to exactly one story: the folding
//! rules run in a fixed order over a claimed-mask, and any change no rule
//! claims lands in the model's single `other` story
//! (§Constraints item 11, "Story coverage totality").
//!
//! Severity is *derived* from the folded changes' directions and a
//! dimension class ([`dimension_class`]), never assigned by a rule
//! directly — this is what keeps `--fail-on downgrade`, the
//! `PropertyDowngrade` diagnostic set, and the risk/cost stories from ever
//! disagreeing (§"Stories" "Severity").

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use super::diff::{BaselineInfo, Change, ChangeKind, DiffReport, Dimension, Direction, ModelDiff};
use crate::analysis::source_bounds::BoundResult;
use crate::analysis::walk::{Determinism as Det, Grain};
use crate::contract::ContractPointView;
use crate::maintenance::{RowIdentity, Technique};

/// A story's severity (`docs/specs/property_diff.md` §"Stories"
/// "Severity") — derived from the folded changes' directions and dimension
/// classes, never assigned by a folding rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Risk,
    Cost,
    Improvement,
    Info,
}

/// The kind of story a folding rule produced (`docs/specs/property_diff.md`
/// §"Stories" "Folding rules").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StoryKind {
    MaintenanceLost,
    MaintenanceGained,
    Refusal,
    RowsMayDuplicate,
    RowKey,
    Reads,
    Dependency,
    Technique,
    Contract,
    Probe,
    ColumnSemantics,
    Schema,
    Other,
}

/// One story (`docs/specs/property_diff.md` §"Stories"): a rendering of the
/// changes it folds, never a derivation of its own.
#[derive(Debug, Clone, Serialize)]
pub struct Story {
    pub kind: StoryKind,
    pub severity: Severity,
    /// The column, source, or cell the story is about — empty for a
    /// whole-model story. The editor's anchor (§Surface "Editor").
    pub subject: String,
    pub lead: String,
    pub detail: String,
    /// Indices into the model's `changes`, the changes this story folds.
    pub changes: Vec<usize>,
}

/// Whether a [`Dimension`] is a *guarantee* dimension (a downgrade in it
/// makes a folding story `risk`), a *cost* dimension (a downgrade makes it
/// `cost`), or neither (`docs/specs/property_diff.md` §"Stories"
/// "Severity"). Exhaustive over [`Dimension`] — a new variant with no arm
/// here is a compile error, never a silent default.
#[derive(PartialEq, Eq)]
enum DimClass {
    Guarantee,
    Cost,
    Neither,
}

fn dimension_class(d: Dimension) -> DimClass {
    match d {
        Dimension::MaintenanceLost
        | Dimension::Grain
        | Dimension::RowIdentity
        | Dimension::CellRowIdentity
        | Dimension::FanOutJoin
        | Dimension::SetOpBarrier
        | Dimension::RefusalAdded
        | Dimension::ContractPoint
        | Dimension::ProbeRemoved
        | Dimension::Determinism
        | Dimension::Comparability => DimClass::Guarantee,
        Dimension::SourceBound
        | Dimension::CellAdded
        | Dimension::CellRemoved
        | Dimension::CellTechnique
        | Dimension::StateDowngrade => DimClass::Cost,
        Dimension::CellCorner
        | Dimension::RefusalRemoved
        | Dimension::ProbeAdded
        | Dimension::ColumnAdded
        | Dimension::ColumnRemoved
        | Dimension::Discriminant
        | Dimension::FdAdded
        | Dimension::FdRemoved
        | Dimension::LiteralColumn
        | Dimension::MaintenanceGained => DimClass::Neither,
    }
}

/// A story that folds at least one downgrade in a guarantee dimension is
/// `risk`; one that folds at least one downgrade, none of them in a
/// guarantee dimension, is `cost`; one that folds at least one upgrade and
/// no downgrade is `improvement`; anything else is `info`
/// (`docs/specs/property_diff.md` §"Stories" "Severity").
fn severity_for(cs: &[&Change]) -> Severity {
    let mut has_guarantee_downgrade = false;
    let mut has_downgrade = false;
    let mut has_upgrade = false;
    for c in cs {
        match c.direction {
            Direction::Downgrade => {
                has_downgrade = true;
                if dimension_class(c.dimension) == DimClass::Guarantee {
                    has_guarantee_downgrade = true;
                }
            }
            Direction::Upgrade => has_upgrade = true,
            Direction::Neutral => {}
        }
    }
    if has_guarantee_downgrade {
        Severity::Risk
    } else if has_downgrade {
        Severity::Cost
    } else if has_upgrade {
        Severity::Improvement
    } else {
        Severity::Info
    }
}

fn find_unclaimed(
    changes: &[Change],
    claimed: &[bool],
    pred: impl Fn(&ChangeKind) -> bool,
) -> Option<usize> {
    changes
        .iter()
        .enumerate()
        .find(|(i, c)| !claimed[*i] && pred(&c.kind))
        .map(|(i, _)| i)
}

fn find_all_unclaimed(
    changes: &[Change],
    claimed: &[bool],
    pred: impl Fn(&ChangeKind) -> bool,
) -> Vec<usize> {
    changes
        .iter()
        .enumerate()
        .filter(|(i, c)| !claimed[*i] && pred(&c.kind))
        .map(|(i, _)| i)
        .collect()
}

fn by_ref<'a>(changes: &'a [Change], idxs: &[usize]) -> Vec<&'a Change> {
    idxs.iter().map(|i| &changes[*i]).collect()
}

/// The columns a [`Grain`]'s proven keys cover, in the order they first
/// appear (`docs/specs/property_diff.md` §"Stories": "the key in a
/// `row_key` story is the columns its grain's keys cover, in the profile's
/// order").
fn grain_columns(g: &Grain) -> Vec<String> {
    let mut cols: Vec<String> = Vec::new();
    for key in &g.keys {
        for c in key {
            if !cols.contains(c) {
                cols.push(c.clone());
            }
        }
    }
    cols
}

fn key_paren(cols: &[String]) -> String {
    format!("({})", cols.join(", "))
}

/// `maintenance_lost` (`docs/specs/property_diff.md` §"Stories" "Folding
/// rules"): claims `maintenance_lost`, every `cell_removed`, and every
/// `refusal_added`.
fn rule_maintenance_lost(changes: &[Change], claimed: &mut [bool]) -> Option<Story> {
    let ml_idx = find_unclaimed(changes, claimed, |k| {
        matches!(k, ChangeKind::MaintenanceLost)
    })?;
    claimed[ml_idx] = true;
    let mut idxs = vec![ml_idx];
    for i in find_all_unclaimed(changes, claimed, |k| {
        matches!(k, ChangeKind::CellRemoved { .. })
    }) {
        claimed[i] = true;
        idxs.push(i);
    }
    let mut detail = "Every run rebuilds the whole table.".to_string();
    for i in find_all_unclaimed(changes, claimed, |k| {
        matches!(k, ChangeKind::RefusalAdded(_))
    }) {
        claimed[i] = true;
        if let ChangeKind::RefusalAdded(r) = &changes[i].kind {
            detail.push_str(&format!(" Reason: {}.", r.text));
        }
        idxs.push(i);
    }
    Some(Story {
        kind: StoryKind::MaintenanceLost,
        severity: severity_for(&by_ref(changes, &idxs)),
        subject: String::new(),
        lead: "No longer incrementally maintained".to_string(),
        detail,
        changes: idxs,
    })
}

/// `maintenance_gained`: the symmetric partner.
fn rule_maintenance_gained(changes: &[Change], claimed: &mut [bool]) -> Option<Story> {
    let mg_idx = find_unclaimed(changes, claimed, |k| {
        matches!(k, ChangeKind::MaintenanceGained)
    })?;
    claimed[mg_idx] = true;
    let mut idxs = vec![mg_idx];
    let added = find_all_unclaimed(changes, claimed, |k| {
        matches!(k, ChangeKind::CellAdded { .. })
    });
    let n = added.len();
    for i in added {
        claimed[i] = true;
        idxs.push(i);
    }
    for i in find_all_unclaimed(changes, claimed, |k| {
        matches!(k, ChangeKind::RefusalRemoved(_))
    }) {
        claimed[i] = true;
        idxs.push(i);
    }
    Some(Story {
        kind: StoryKind::MaintenanceGained,
        severity: severity_for(&by_ref(changes, &idxs)),
        subject: String::new(),
        lead: "Now incrementally maintained".to_string(),
        detail: format!("{n} maintenance cell(s) admitted."),
        changes: idxs,
    })
}

/// `refusal`: one story per remaining `refusal_added`/`refusal_removed`.
fn rule_refusal(changes: &[Change], claimed: &mut [bool]) -> Vec<Story> {
    let mut stories = Vec::new();
    for i in find_all_unclaimed(changes, claimed, |k| {
        matches!(
            k,
            ChangeKind::RefusalAdded(_) | ChangeKind::RefusalRemoved(_)
        )
    }) {
        claimed[i] = true;
        let (lead, text) = match &changes[i].kind {
            ChangeKind::RefusalAdded(r) => ("Maintenance refused", r.text.clone()),
            ChangeKind::RefusalRemoved(r) => ("Refusal cleared", r.text.clone()),
            _ => unreachable!("filtered to refusal changes"),
        };
        stories.push(Story {
            kind: StoryKind::Refusal,
            severity: severity_for(&[&changes[i]]),
            subject: String::new(),
            lead: lead.to_string(),
            detail: format!("{text}."),
            changes: vec![i],
        });
    }
    stories
}

/// `rows_may_duplicate`: fires when `fan_out_join` went `false` → `true` or
/// `row_identity` went `Key` → `WholeRow`; claims those, `grain`, every
/// `cell_row_identity`, and every `fd_added`/`fd_removed`.
fn rule_rows_may_duplicate(changes: &[Change], claimed: &mut [bool]) -> Option<Story> {
    let fan_idx = find_unclaimed(
        changes,
        claimed,
        |k| matches!(k, ChangeKind::FanOutJoin { old, new } if !*old && *new),
    );
    let identity_idx = find_unclaimed(changes, claimed, |k| {
        matches!(k, ChangeKind::RowIdentity { old, new }
            if matches!(old.identity, RowIdentity::Key(_))
                && matches!(new.identity, RowIdentity::WholeRow))
    });
    if fan_idx.is_none() && identity_idx.is_none() {
        return None;
    }
    let mut idxs = Vec::new();
    if let Some(i) = fan_idx {
        claimed[i] = true;
        idxs.push(i);
    }
    if let Some(i) = identity_idx {
        claimed[i] = true;
        idxs.push(i);
    }
    let grain_idx = find_unclaimed(changes, claimed, |k| matches!(k, ChangeKind::Grain { .. }));
    let old_key_display = if let Some(gi) = grain_idx {
        claimed[gi] = true;
        idxs.push(gi);
        match &changes[gi].kind {
            ChangeKind::Grain { old, .. } => key_paren(&grain_columns(old)),
            _ => String::new(),
        }
    } else if let Some(ii) = identity_idx {
        match &changes[ii].kind {
            ChangeKind::RowIdentity { old, .. } => match &old.identity {
                RowIdentity::Key(cols) => key_paren(cols),
                RowIdentity::WholeRow => String::new(),
            },
            _ => String::new(),
        }
    } else {
        String::new()
    };
    for i in find_all_unclaimed(changes, claimed, |k| {
        matches!(k, ChangeKind::CellRowIdentity { .. })
    }) {
        claimed[i] = true;
        idxs.push(i);
    }
    for i in find_all_unclaimed(changes, claimed, |k| {
        matches!(k, ChangeKind::FdAdded(_) | ChangeKind::FdRemoved(_))
    }) {
        claimed[i] = true;
        idxs.push(i);
    }
    Some(Story {
        kind: StoryKind::RowsMayDuplicate,
        severity: severity_for(&by_ref(changes, &idxs)),
        subject: String::new(),
        lead: "Rows may be duplicated".to_string(),
        detail: format!(
            "A join can now match more than one row per {old_key_display}, so smelt can no longer identify a row by its key."
        ),
        changes: idxs,
    })
}

/// `grain`/`row_identity`/`cell_row_identity`/`fd_*` classification shared
/// by `row_key` (`docs/specs/property_diff.md` §"Stories" "Folding rules").
fn classify_row_key(
    changes: &[Change],
    grain_idx: Option<usize>,
    identity_idx: Option<usize>,
) -> (String, String) {
    if let Some(gi) = grain_idx {
        if let ChangeKind::Grain { old, new, .. } = &changes[gi].kind {
            let old_has = !old.keys.is_empty();
            let new_has = !new.keys.is_empty();
            let old_cols = grain_columns(old);
            let new_cols = grain_columns(new);
            if old_has && !new_has {
                return (
                    "Row key lost".to_string(),
                    format!("No longer proves one row per {}.", key_paren(&old_cols)),
                );
            }
            if !old_has && new_has {
                return (
                    "Row key proven".to_string(),
                    format!("Now proves one row per {}.", key_paren(&new_cols)),
                );
            }
            let old_set: BTreeSet<&String> = old_cols.iter().collect();
            let new_set: BTreeSet<&String> = new_cols.iter().collect();
            if old_set != new_set {
                if old_set.is_subset(&new_set) {
                    return (
                        "Row key widened".to_string(),
                        format!(
                            "Rows are now unique per {}, no longer per {}; downstream joins on the old key may fan out.",
                            key_paren(&new_cols),
                            key_paren(&old_cols)
                        ),
                    );
                }
                if new_set.is_subset(&old_set) {
                    return (
                        "Row key narrowed".to_string(),
                        format!(
                            "Rows are now unique per {}, a stronger claim than {}.",
                            key_paren(&new_cols),
                            key_paren(&old_cols)
                        ),
                    );
                }
                return (
                    "Row key changed".to_string(),
                    format!(
                        "Was {}, now {}.",
                        key_paren(&old_cols),
                        key_paren(&new_cols)
                    ),
                );
            }
        }
    }
    if let Some(ii) = identity_idx {
        if let ChangeKind::RowIdentity { old, new } = &changes[ii].kind {
            match (&old.identity, &new.identity) {
                (RowIdentity::WholeRow, RowIdentity::Key(cols)) => {
                    return (
                        "Row key proven".to_string(),
                        format!("Now proves one row per {}.", key_paren(cols)),
                    );
                }
                (RowIdentity::Key(oc), RowIdentity::Key(nc)) if oc != nc => {
                    return (
                        "Row key changed".to_string(),
                        format!("Was {}, now {}.", key_paren(oc), key_paren(nc)),
                    );
                }
                _ => {}
            }
        }
    }
    (
        "Row key changed".to_string(),
        "Row key changed.".to_string(),
    )
}

/// `row_key`: claims `grain`, `row_identity`, every `cell_row_identity`,
/// and every `fd_added`/`fd_removed` still unclaimed after
/// `rows_may_duplicate` has run.
fn rule_row_key(changes: &[Change], claimed: &mut [bool]) -> Option<Story> {
    let grain_idx = find_unclaimed(changes, claimed, |k| matches!(k, ChangeKind::Grain { .. }));
    let identity_idx = find_unclaimed(changes, claimed, |k| {
        matches!(k, ChangeKind::RowIdentity { .. })
    });
    let cri_idxs = find_all_unclaimed(changes, claimed, |k| {
        matches!(k, ChangeKind::CellRowIdentity { .. })
    });
    let fd_idxs = find_all_unclaimed(changes, claimed, |k| {
        matches!(k, ChangeKind::FdAdded(_) | ChangeKind::FdRemoved(_))
    });
    if grain_idx.is_none() && identity_idx.is_none() && cri_idxs.is_empty() && fd_idxs.is_empty() {
        return None;
    }
    let (lead, detail) = classify_row_key(changes, grain_idx, identity_idx);
    let mut idxs = Vec::new();
    if let Some(i) = grain_idx {
        claimed[i] = true;
        idxs.push(i);
    }
    if let Some(i) = identity_idx {
        claimed[i] = true;
        idxs.push(i);
    }
    for i in cri_idxs {
        claimed[i] = true;
        idxs.push(i);
    }
    for i in fd_idxs {
        claimed[i] = true;
        idxs.push(i);
    }
    Some(Story {
        kind: StoryKind::RowKey,
        severity: severity_for(&by_ref(changes, &idxs)),
        subject: String::new(),
        lead,
        detail,
        changes: idxs,
    })
}

fn technique_display(t: Technique) -> String {
    serde_json::to_value(t)
        .ok()
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| format!("{t:?}"))
}

/// `dependency`: for each trigger source appearing only among `cell_added`
/// changes, or only among `cell_removed` changes, folds those cells and
/// that source's `source_bound` change when it too is one-sided; a matched
/// cell's `cell_removed` graded a downgrade because its source survives
/// gets its own "maintenance route lost" story
/// (`docs/specs/property_diff.md` §"Stories" "Folding rules").
fn rule_dependency(changes: &[Change], claimed: &mut [bool]) -> Vec<Story> {
    let mut stories = Vec::new();
    let mut added_by_source: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    let mut removed_by_source: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, c) in changes.iter().enumerate() {
        if claimed[i] {
            continue;
        }
        match &c.kind {
            ChangeKind::CellAdded { new, .. } => {
                if let Some(src) = &new.trigger_source {
                    added_by_source.entry(src.clone()).or_default().push(i);
                }
            }
            ChangeKind::CellRemoved { old, .. } => {
                if let Some(src) = &old.trigger_source {
                    removed_by_source.entry(src.clone()).or_default().push(i);
                }
            }
            _ => {}
        }
    }
    let mut sources: BTreeSet<String> = added_by_source.keys().cloned().collect();
    sources.extend(removed_by_source.keys().cloned());
    for source in sources {
        let added_idxs = added_by_source.get(&source).cloned().unwrap_or_default();
        let removed_idxs = removed_by_source.get(&source).cloned().unwrap_or_default();
        if !added_idxs.is_empty() && removed_idxs.is_empty() {
            let mut idxs = added_idxs.clone();
            for i in &idxs {
                claimed[*i] = true;
            }
            if let Some(bi) = find_unclaimed(
                changes,
                claimed,
                |k| matches!(k, ChangeKind::SourceBound { source: s, .. } if s == &source),
            ) {
                claimed[bi] = true;
                idxs.push(bi);
            }
            let all_local = added_idxs.iter().all(|i| {
                matches!(&changes[*i].kind, ChangeKind::CellAdded { new, .. } if new.partition_local)
            });
            let (lead, detail) = if all_local {
                let technique = match &changes[added_idxs[0]].kind {
                    ChangeKind::CellAdded { new, .. } => technique_display(new.technique),
                    _ => String::new(),
                };
                (
                    "New dependency".to_string(),
                    format!("Changes to {source} are applied by {technique}."),
                )
            } else {
                (
                    "New dependency read in full".to_string(),
                    format!(
                        "{source} has no time column, so every run reads all of it, and any change to it rebuilds the whole model."
                    ),
                )
            };
            stories.push(Story {
                kind: StoryKind::Dependency,
                severity: severity_for(&by_ref(changes, &idxs)),
                subject: source.clone(),
                lead,
                detail,
                changes: idxs,
            });
        } else if !removed_idxs.is_empty() && added_idxs.is_empty() {
            // Fold only the removed cells graded non-downgrade here (their
            // source was dropped altogether). A removed cell graded a
            // downgrade — its source still survives via another, unchanged
            // sibling cell that never shows up as `cell_added` — is left
            // unclaimed for the per-cell "Maintenance route lost" pass below
            // (`docs/specs/property_diff.md` §"Direction"
            // "cell_added"/"cell_removed" row).
            let mut idxs: Vec<usize> = removed_idxs
                .iter()
                .copied()
                .filter(|i| changes[*i].direction != Direction::Downgrade)
                .collect();
            if idxs.is_empty() {
                continue;
            }
            for i in &idxs {
                claimed[*i] = true;
            }
            if let Some(bi) = find_unclaimed(
                changes,
                claimed,
                |k| matches!(k, ChangeKind::SourceBound { source: s, .. } if s == &source),
            ) {
                claimed[bi] = true;
                idxs.push(bi);
            }
            stories.push(Story {
                kind: StoryKind::Dependency,
                severity: severity_for(&by_ref(changes, &idxs)),
                subject: source.clone(),
                lead: "Dependency removed".to_string(),
                detail: format!("No longer reads {source}."),
                changes: idxs,
            });
        }
        // A source appearing in both sets is not one-sided: its cell_added/
        // cell_removed changes are left for the per-cell pass below (or,
        // failing that, `other`) — `docs/specs/property_diff.md` §"Stories"
        // "Folding rules" claims only one-sided sources here.
    }
    for i in find_all_unclaimed(changes, claimed, |k| {
        matches!(k, ChangeKind::CellRemoved { .. })
    }) {
        if changes[i].direction != Direction::Downgrade {
            continue;
        }
        claimed[i] = true;
        if let ChangeKind::CellRemoved { old, .. } = &changes[i].kind {
            let group = old.group.clone();
            let source = old.trigger_source.clone().unwrap_or_default();
            stories.push(Story {
                kind: StoryKind::Dependency,
                severity: severity_for(&[&changes[i]]),
                subject: source.clone(),
                lead: "Maintenance route lost".to_string(),
                detail: format!(
                    "{group} no longer has a maintenance route for changes from {source}."
                ),
                changes: vec![i],
            });
        }
    }
    stories
}

fn humanise_window(b: &BoundResult) -> String {
    match b {
        BoundResult::Bounded { before, after, .. } => {
            if before.0 == after.0 {
                format!(
                    "{} either side of the run window",
                    humanise_seconds(before.0)
                )
            } else {
                format!(
                    "{} before and {} after the run window",
                    humanise_seconds(before.0),
                    humanise_seconds(after.0)
                )
            }
        }
        BoundResult::Unbounded | BoundResult::NotDerivable => "all history".to_string(),
    }
}

/// `reads`: every remaining `source_bound`, one story per distinct
/// `(old, new)` rendered-window pair, listing the sources that share it.
fn rule_reads(changes: &[Change], claimed: &mut [bool]) -> Vec<Story> {
    let idxs = find_all_unclaimed(changes, claimed, |k| {
        matches!(k, ChangeKind::SourceBound { .. })
    });
    if idxs.is_empty() {
        return Vec::new();
    }
    let mut groups: BTreeMap<(String, String), Vec<usize>> = BTreeMap::new();
    for i in idxs {
        if let ChangeKind::SourceBound { old, new, .. } = &changes[i].kind {
            let key = (humanise_window(old), humanise_window(new));
            groups.entry(key).or_default().push(i);
        }
    }
    let mut stories = Vec::new();
    for ((old_w, new_w), group_idxs) in groups {
        for i in &group_idxs {
            claimed[*i] = true;
        }
        let mut sources: Vec<String> = group_idxs
            .iter()
            .map(|i| changes[*i].subject.clone())
            .collect();
        sources.sort();
        let lead = if changes[group_idxs[0]].direction == Direction::Upgrade {
            "Reads less per run"
        } else {
            "Reads more per run"
        };
        let detail = format!(
            "Each run now reads {new_w} of {} (was {old_w}).",
            sources.join(", ")
        );
        stories.push(Story {
            kind: StoryKind::Reads,
            severity: severity_for(&by_ref(changes, &group_idxs)),
            subject: String::new(),
            lead: lead.to_string(),
            detail,
            changes: group_idxs,
        });
    }
    stories
}

/// The one place `technique` sentences name a trigger source
/// (`docs/specs/property_diff.md` §"Stories" "Folding rules", `technique`
/// row): reads the structured `trigger_source`/`group` the matched
/// [`crate::analysis::profile::CellVerdict`] carried into the change
/// (`docs/specs/property_diff.md` §"The property profile" item 2), never a
/// value recovered by re-parsing the cell's own `{group}@{trigger:?}` join
/// key or its `Trigger` debug text (`CLAUDE.md` §"Source-derived
/// projection"). `trigger_source` is `None` for `Backfill`/`ColumnAdded`
/// triggers, which have no per-source address.
fn changes_applied_sentence(
    trigger_source: &Option<String>,
    group: &str,
    new_t: Technique,
    old_t: Technique,
) -> String {
    match trigger_source {
        Some(source) => format!(
            "Changes from {source} are applied to {group} by {} instead of {}.",
            technique_display(new_t),
            technique_display(old_t)
        ),
        None => format!(
            "Changes are applied to {group} by {} instead of {}.",
            technique_display(new_t),
            technique_display(old_t)
        ),
    }
}

/// `technique`: one story per `cell_technique`, plus that cell's
/// `state_downgrade` and `cell_corner`.
fn rule_technique(changes: &[Change], claimed: &mut [bool]) -> Vec<Story> {
    let mut stories = Vec::new();
    for i in find_all_unclaimed(changes, claimed, |k| {
        matches!(k, ChangeKind::CellTechnique { .. })
    }) {
        claimed[i] = true;
        let (cell_key, group, trigger_source, old_t, new_t) = match &changes[i].kind {
            ChangeKind::CellTechnique {
                cell,
                group,
                trigger_source,
                old,
                new,
            } => (
                cell.clone(),
                group.clone(),
                trigger_source.clone(),
                *old,
                *new,
            ),
            _ => unreachable!("filtered to cell_technique changes"),
        };
        let dir = changes[i].direction;
        let mut idxs = vec![i];
        let mut detail = changes_applied_sentence(&trigger_source, &group, new_t, old_t);
        if let Some(si) = find_unclaimed(
            changes,
            claimed,
            |k| matches!(k, ChangeKind::StateDowngrade { cell, .. } if cell == &cell_key),
        ) {
            claimed[si] = true;
            idxs.push(si);
            if let ChangeKind::StateDowngrade { old, new, .. } = &changes[si].kind {
                if let Some(reason) = new.as_ref().or(old.as_ref()).map(|sd| sd.reason.clone()) {
                    detail.push_str(&format!(" Reason: {reason}."));
                }
            }
        }
        if let Some(ci) = find_unclaimed(
            changes,
            claimed,
            |k| matches!(k, ChangeKind::CellCorner { cell, .. } if cell == &cell_key),
        ) {
            claimed[ci] = true;
            idxs.push(ci);
        }
        let lead = if dir == Direction::Downgrade {
            "Costlier maintenance"
        } else {
            "Cheaper maintenance"
        };
        stories.push(Story {
            kind: StoryKind::Technique,
            severity: severity_for(&by_ref(changes, &idxs)),
            subject: cell_key,
            lead: lead.to_string(),
            detail,
            changes: idxs,
        });
    }
    stories
}

fn contract_point_display(v: &ContractPointView) -> String {
    if v.is_default() {
        return "default".to_string();
    }
    let mut parts = Vec::new();
    if let Some(fh) = &v.frozen_horizon {
        parts.push(format!("frozen_horizon {fh}"));
    }
    if let Some(d) = &v.deferral {
        match v.deferral_origin.as_deref() {
            Some("cell") => parts.push(format!("deferral {d} (cell)")),
            _ => parts.push(format!("deferral {d}")),
        }
    }
    if let Some(rd) = &v.retain_departed {
        if rd == "true" {
            parts.push("retain_departed".to_string());
        } else {
            parts.push(format!("retain_departed ({rd})"));
        }
    }
    parts.join(", ")
}

/// `contract`: one story per remaining `contract_point`.
fn rule_contract(changes: &[Change], claimed: &mut [bool]) -> Vec<Story> {
    let mut stories = Vec::new();
    for i in find_all_unclaimed(changes, claimed, |k| {
        matches!(k, ChangeKind::ContractPoint { .. })
    }) {
        claimed[i] = true;
        if let ChangeKind::ContractPoint { cell, old, new } = &changes[i].kind {
            let lead = match changes[i].direction {
                Direction::Downgrade => "Contract relaxed",
                Direction::Upgrade => "Contract tightened",
                Direction::Neutral => "Contract changed",
            };
            stories.push(Story {
                kind: StoryKind::Contract,
                severity: severity_for(&[&changes[i]]),
                subject: cell.clone(),
                lead: lead.to_string(),
                detail: format!(
                    "{cell} moved from {} to {}.",
                    contract_point_display(old),
                    contract_point_display(new)
                ),
                changes: vec![i],
            });
        }
    }
    stories
}

/// `probe`: one story per remaining `probe_added`/`probe_removed`.
fn rule_probe(changes: &[Change], claimed: &mut [bool]) -> Vec<Story> {
    let mut stories = Vec::new();
    for i in find_all_unclaimed(changes, claimed, |k| {
        matches!(k, ChangeKind::ProbeAdded(_) | ChangeKind::ProbeRemoved(_))
    }) {
        claimed[i] = true;
        let (lead, fact, verb) = match &changes[i].kind {
            ChangeKind::ProbeRemoved(p) => (
                "Runtime check removed",
                p.fact.clone(),
                "is no longer checked",
            ),
            ChangeKind::ProbeAdded(p) => ("Runtime check added", p.fact.clone(), "is now checked"),
            _ => unreachable!("filtered to probe changes"),
        };
        stories.push(Story {
            kind: StoryKind::Probe,
            severity: severity_for(&[&changes[i]]),
            subject: fact.clone(),
            lead: lead.to_string(),
            detail: format!("Declared fact {fact} {verb} at run time."),
            changes: vec![i],
        });
    }
    stories
}

/// `column_semantics`: `determinism`/`comparability` on a column present on
/// both sides (matched, not added/removed — that is `schema`'s job).
fn rule_column_semantics(changes: &[Change], claimed: &mut [bool]) -> Vec<Story> {
    let mut stories = Vec::new();
    for i in find_all_unclaimed(changes, claimed, |k| {
        matches!(
            k,
            ChangeKind::Determinism {
                old: Some(_),
                new: Some(_),
                ..
            }
        ) || matches!(
            k,
            ChangeKind::Comparability {
                old: Some(_),
                new: Some(_),
                ..
            }
        )
    }) {
        claimed[i] = true;
        let (lead, detail, subject) = match &changes[i].kind {
            ChangeKind::Determinism { column, old, new } => {
                let (Some(o), Some(n)) = (old, new) else {
                    unreachable!("filtered to both-present determinism changes")
                };
                if n > o {
                    (
                        "Column now nondeterministic".to_string(),
                        format!("{column} is now {n:?}-nondeterministic (was {o:?})."),
                        column.clone(),
                    )
                } else {
                    (
                        "Column now deterministic".to_string(),
                        format!("{column} is now deterministic (was {o:?})."),
                        column.clone(),
                    )
                }
            }
            ChangeKind::Comparability { column, old, new } => {
                let (Some(o), Some(n)) = (old, new) else {
                    unreachable!("filtered to both-present comparability changes")
                };
                if n > o {
                    (
                        "Column no longer comparable".to_string(),
                        format!("{column} can no longer be compared between runs."),
                        column.clone(),
                    )
                } else {
                    (
                        "Column now comparable".to_string(),
                        format!("{column} can now be compared between runs."),
                        column.clone(),
                    )
                }
            }
            _ => unreachable!("filtered to determinism/comparability changes"),
        };
        stories.push(Story {
            kind: StoryKind::ColumnSemantics,
            severity: severity_for(&[&changes[i]]),
            subject,
            lead,
            detail,
            changes: vec![i],
        });
    }
    stories
}

fn column_of(k: &ChangeKind) -> Option<&String> {
    match k {
        ChangeKind::Determinism { column, .. }
        | ChangeKind::Comparability { column, .. }
        | ChangeKind::Discriminant { column, .. }
        | ChangeKind::LiteralColumn { column, .. } => Some(column),
        _ => None,
    }
}

/// `schema`: every `column_added`/`column_removed`, plus each such column's
/// `determinism`, `comparability`, `discriminant`, and `literal_column`.
fn rule_schema(changes: &[Change], claimed: &mut [bool]) -> Option<Story> {
    let added_idxs = find_all_unclaimed(changes, claimed, |k| {
        matches!(k, ChangeKind::ColumnAdded(_))
    });
    let removed_idxs = find_all_unclaimed(changes, claimed, |k| {
        matches!(k, ChangeKind::ColumnRemoved(_))
    });
    if added_idxs.is_empty() && removed_idxs.is_empty() {
        return None;
    }
    let mut idxs = Vec::new();
    let mut added_names: Vec<String> = Vec::new();
    for i in &added_idxs {
        claimed[*i] = true;
        idxs.push(*i);
        if let ChangeKind::ColumnAdded(n) = &changes[*i].kind {
            added_names.push(n.clone());
        }
    }
    let mut removed_names: Vec<String> = Vec::new();
    for i in &removed_idxs {
        claimed[*i] = true;
        idxs.push(*i);
        if let ChangeKind::ColumnRemoved(n) = &changes[*i].kind {
            removed_names.push(n.clone());
        }
    }
    let touched: BTreeSet<&String> = added_names.iter().chain(removed_names.iter()).collect();
    let extra_idxs = find_all_unclaimed(changes, claimed, |k| {
        column_of(k).is_some_and(|c| touched.contains(c))
    });
    for i in &extra_idxs {
        claimed[*i] = true;
        idxs.push(*i);
    }
    let mut notes = Vec::new();
    for i in &extra_idxs {
        if let ChangeKind::Determinism { column, new, .. } = &changes[*i].kind {
            if added_names.contains(column) {
                if let Some(level) = new {
                    if *level != Det::Clean {
                        notes.push(format!("{column} is {level:?}-nondeterministic."));
                    }
                }
            }
        }
    }
    let mut parts = Vec::new();
    if !added_names.is_empty() {
        parts.push(format!("Adds {}", added_names.join(", ")));
    }
    if !removed_names.is_empty() {
        parts.push(format!("removes {}", removed_names.join(", ")));
    }
    let mut detail = format!("{}.", parts.join("; "));
    for note in notes {
        detail.push_str(&format!(" ({note})"));
    }
    Some(Story {
        kind: StoryKind::Schema,
        severity: severity_for(&by_ref(changes, &idxs)),
        subject: String::new(),
        lead: "Schema".to_string(),
        detail,
        changes: idxs,
    })
}

fn dimension_label(d: Dimension) -> String {
    serde_json::to_value(d)
        .ok()
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "unknown".to_string())
}

/// `other`: everything unclaimed, one story per model
/// (`docs/specs/property_diff.md` §Constraints item 11, "Story coverage
/// totality") — never dropped.
fn rule_other(changes: &[Change], claimed: &mut [bool]) -> Option<Story> {
    let idxs = find_all_unclaimed(changes, claimed, |_| true);
    if idxs.is_empty() {
        return None;
    }
    for i in &idxs {
        claimed[*i] = true;
    }
    let detail = idxs
        .iter()
        .map(|i| {
            let c = &changes[*i];
            if c.subject.is_empty() {
                dimension_label(c.dimension)
            } else {
                format!("{} {}", dimension_label(c.dimension), c.subject)
            }
        })
        .collect::<Vec<_>>()
        .join("; ");
    Some(Story {
        kind: StoryKind::Other,
        severity: severity_for(&by_ref(changes, &idxs)),
        subject: String::new(),
        lead: "Also changed".to_string(),
        detail,
        changes: idxs,
    })
}

fn severity_rank(s: Severity) -> u8 {
    match s {
        Severity::Risk => 0,
        Severity::Cost => 1,
        Severity::Improvement => 2,
        Severity::Info => 3,
    }
}

/// Fold a shifted model's `changes` into severity-ranked stories
/// (`docs/specs/property_diff.md` §"Stories"). Stories are ordered by
/// severity (`risk`, `cost`, `improvement`, `info`), then by the rules'
/// fixed order (`sort_by_key` is stable, so ties preserve the order the
/// rules ran in).
pub fn narrate(model: &ModelDiff) -> Vec<Story> {
    let changes = &model.changes;
    let mut claimed = vec![false; changes.len()];
    let mut stories = Vec::new();
    if let Some(s) = rule_maintenance_lost(changes, &mut claimed) {
        stories.push(s);
    }
    if let Some(s) = rule_maintenance_gained(changes, &mut claimed) {
        stories.push(s);
    }
    stories.extend(rule_refusal(changes, &mut claimed));
    if let Some(s) = rule_rows_may_duplicate(changes, &mut claimed) {
        stories.push(s);
    }
    if let Some(s) = rule_row_key(changes, &mut claimed) {
        stories.push(s);
    }
    stories.extend(rule_dependency(changes, &mut claimed));
    stories.extend(rule_reads(changes, &mut claimed));
    stories.extend(rule_technique(changes, &mut claimed));
    stories.extend(rule_contract(changes, &mut claimed));
    stories.extend(rule_probe(changes, &mut claimed));
    stories.extend(rule_column_semantics(changes, &mut claimed));
    if let Some(s) = rule_schema(changes, &mut claimed) {
        stories.push(s);
    }
    if let Some(s) = rule_other(changes, &mut claimed) {
        stories.push(s);
    }
    stories.sort_by_key(|s| severity_rank(s.severity));
    stories
}

/// A whole number of days, else whole hours, else whole minutes, else
/// seconds — singular when the count is `1`
/// (`docs/specs/property_diff.md` §"Stories").
pub fn humanise_seconds(s: u64) -> String {
    if s > 0 && s.is_multiple_of(86_400) {
        let d = s / 86_400;
        format!("{d} day{}", if d == 1 { "" } else { "s" })
    } else if s > 0 && s.is_multiple_of(3600) {
        let h = s / 3600;
        format!("{h} hour{}", if h == 1 { "" } else { "s" })
    } else if s > 0 && s.is_multiple_of(60) {
        let m = s / 60;
        format!("{m} minute{}", if m == 1 { "" } else { "s" })
    } else {
        format!("{s} second{}", if s == 1 { "" } else { "s" })
    }
}

/// The report's one-line summary (`docs/specs/property_diff.md` §"Stories"
/// "Headline"). Counts are over the reported set — `report.models`, as
/// narrowed by `--select` when the caller has already narrowed it.
pub fn headline(report: &DiffReport) -> String {
    let n = report.models.len();
    let mut clauses: Vec<String> = Vec::new();

    let lost = report
        .models
        .iter()
        .filter(|m| {
            m.stories
                .iter()
                .any(|s| s.kind == StoryKind::MaintenanceLost)
        })
        .count();
    if lost > 0 {
        clauses.push(format!("{lost} lost incremental maintenance"));
    }
    let risk = report
        .models
        .iter()
        .filter(|m| {
            m.stories
                .iter()
                .any(|s| s.severity == Severity::Risk && s.kind != StoryKind::MaintenanceLost)
        })
        .count();
    if risk > 0 {
        clauses.push(format!("{risk} with correctness risks"));
    }
    let cost = report
        .models
        .iter()
        .filter(|m| m.stories.iter().any(|s| s.severity == Severity::Cost))
        .count();
    if cost > 0 {
        clauses.push(format!("{cost} read more per run"));
    }
    let improved = report
        .models
        .iter()
        .filter(|m| {
            m.stories
                .iter()
                .any(|s| s.severity == Severity::Improvement)
                && !m
                    .stories
                    .iter()
                    .any(|s| matches!(s.severity, Severity::Risk | Severity::Cost))
        })
        .count();
    if improved > 0 {
        clauses.push(format!("{improved} improved"));
    }

    let mut out = format!("{n} model(s) shifted");
    for c in &clauses {
        out.push_str(" · ");
        out.push_str(c);
    }
    if report.summary.downgrades == 0 {
        out.push_str(" · no downgrades");
    }
    out
}

/// Per-model: `<r> risk(s), <c> costlier vs <short ref>`, a zero term
/// omitted; `changed vs <short ref>` when the model has neither
/// (`docs/specs/property_diff.md` §"Stories" "Lens title").
pub fn lens_title(model: &ModelDiff, baseline: &BaselineInfo) -> String {
    let r = model
        .stories
        .iter()
        .filter(|s| s.severity == Severity::Risk)
        .count();
    let c = model
        .stories
        .iter()
        .filter(|s| s.severity == Severity::Cost)
        .count();
    let short = super::diff_render::short_ref(baseline);
    let mut parts = Vec::new();
    if r > 0 {
        parts.push(format!("{r} risk{}", if r == 1 { "" } else { "s" }));
    }
    if c > 0 {
        parts.push(format!("{c} costlier"));
    }
    if parts.is_empty() {
        format!("changed vs {short}")
    } else {
        format!("{} vs {short}", parts.join(", "))
    }
}

/// `[risk]`/`[cost]`/`[improved]`/`[info]` (`docs/specs/property_diff.md`
/// §Surface "Text").
pub fn severity_label(s: Severity) -> &'static str {
    match s {
        Severity::Risk => "risk",
        Severity::Cost => "cost",
        Severity::Improvement => "improved",
        Severity::Info => "info",
    }
}

/// 🔴/⚠️/🟢/ℹ️ (`docs/specs/property_diff.md` §Surface "Markdown").
pub fn severity_glyph(s: Severity) -> &'static str {
    match s {
        Severity::Risk => "🔴",
        Severity::Cost => "⚠️",
        Severity::Improvement => "🟢",
        Severity::Info => "ℹ️",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::diff::{Cause, CauseKind};
    use crate::analysis::profile::{CellVerdict, ProfileRefusal};
    use crate::analysis::source_bounds::Seconds;
    use crate::analysis::walk::{DerivedFd, Grain};
    use crate::contract::ContractPointView;
    use crate::maintenance::availability::StateDowngrade;
    use crate::maintenance::availability::StateStructure;
    use crate::maintenance::{RowIdentityVerdict, Technique};

    fn cell_verdict(
        group: &str,
        source: &str,
        technique: Technique,
        partition_local: bool,
    ) -> CellVerdict {
        CellVerdict {
            group: group.to_string(),
            trigger: format!("NewData {{ source: {source:?} }}"),
            corner: "RecomputeRegion".to_string(),
            technique,
            row_identity: RowIdentityVerdict {
                identity: RowIdentity::WholeRow,
                proven_mismatch: None,
            },
            contract_point: ContractPointView::default(),
            state_downgrade: None,
            trigger_source: Some(source.to_string()),
            partition_local,
            locality_reason: if partition_local {
                None
            } else {
                Some((
                    source.to_string(),
                    "no partition_column declared".to_string(),
                ))
            },
        }
    }

    fn model_with(changes: Vec<ChangeKind>) -> ModelDiff {
        ModelDiff {
            model: "gold.eventstream_with_identity".to_string(),
            cause: Cause {
                kind: CauseKind::Edited,
                of: vec![],
                reason: None,
            },
            changes: changes.into_iter().map(Change::from_kind).collect(),
            stories: Vec::new(),
        }
    }

    fn bounded(days: u64) -> BoundResult {
        BoundResult::Bounded {
            source_partition_col: "event_date".to_string(),
            before: Seconds(days * 86_400),
            after: Seconds(days * 86_400),
        }
    }

    #[test]
    fn humanise_seconds_renders_the_largest_whole_unit() {
        assert_eq!(humanise_seconds(86_400), "1 day");
        assert_eq!(humanise_seconds(604_800), "7 days");
        assert_eq!(humanise_seconds(3600), "1 hour");
        assert_eq!(humanise_seconds(90), "90 seconds");
        assert_eq!(humanise_seconds(5400), "90 minutes");
    }

    #[test]
    fn widened_window_folds_into_one_reads_story() {
        let device_cell = cell_verdict(
            "{device_type}",
            "raw.devices",
            Technique::DeleteInsert,
            false,
        );
        let model = model_with(vec![
            ChangeKind::SourceBound {
                source: "gold.identity_forward_only".to_string(),
                old: bounded(1),
                new: bounded(7),
            },
            ChangeKind::SourceBound {
                source: "silver.sessions".to_string(),
                old: bounded(1),
                new: bounded(7),
            },
            ChangeKind::ColumnAdded("device_type".to_string()),
            ChangeKind::Determinism {
                column: "device_type".to_string(),
                old: None,
                new: Some(Det::Clean),
            },
            ChangeKind::Comparability {
                column: "device_type".to_string(),
                old: None,
                new: Some(crate::analysis::walk::Comparability::Comparable),
            },
            ChangeKind::CellAdded {
                cell: "{device_type}@NewData".to_string(),
                new: Box::new(device_cell),
                still_maintained: true,
            },
        ]);
        let stories = narrate(&model);
        let kinds: Vec<StoryKind> = stories.iter().map(|s| s.kind).collect();
        assert_eq!(
            kinds,
            vec![StoryKind::Dependency, StoryKind::Reads, StoryKind::Schema]
        );
        assert_eq!(stories[0].severity, Severity::Cost);
        assert!(stories[0].detail.contains("raw.devices"));
        assert_eq!(stories[1].severity, Severity::Cost);
        assert_eq!(
            stories[1].detail,
            "Each run now reads 7 days either side of the run window of gold.identity_forward_only, silver.sessions (was 1 day either side of the run window)."
        );
        assert!(!stories[1].detail.contains("P7D"));
        assert_eq!(stories[2].severity, Severity::Info);
        assert_eq!(stories[2].detail, "Adds device_type.");
    }

    #[test]
    fn fan_out_join_folds_grain_identity_and_fds_into_rows_may_duplicate() {
        let old_grain = Grain {
            keys: vec![vec!["revenue_date".to_string(), "user_id".to_string()]],
        };
        let new_grain = Grain {
            keys: vec![vec![
                "revenue_date".to_string(),
                "user_id".to_string(),
                "user_name".to_string(),
            ]],
        };
        let mut changes = vec![
            ChangeKind::Grain {
                subject: String::new(),
                old: old_grain,
                new: new_grain,
            },
            ChangeKind::RowIdentity {
                old: RowIdentityVerdict {
                    identity: RowIdentity::Key(vec![
                        "revenue_date".to_string(),
                        "user_id".to_string(),
                    ]),
                    proven_mismatch: None,
                },
                new: RowIdentityVerdict {
                    identity: RowIdentity::WholeRow,
                    proven_mismatch: None,
                },
            },
            ChangeKind::FanOutJoin {
                old: false,
                new: true,
            },
            ChangeKind::ColumnAdded("user_name".to_string()),
            ChangeKind::Determinism {
                column: "user_name".to_string(),
                old: None,
                new: Some(Det::Clean),
            },
            ChangeKind::Comparability {
                column: "user_name".to_string(),
                old: None,
                new: Some(crate::analysis::walk::Comparability::Comparable),
            },
        ];
        for i in 0..5 {
            changes.push(ChangeKind::FdRemoved(DerivedFdStub::fd(
                &format!("k{i}"),
                "v",
            )));
            changes.push(ChangeKind::FdAdded(DerivedFdStub::fd(
                &format!("k{i}"),
                "v2",
            )));
        }
        let model = model_with(changes);
        let stories = narrate(&model);
        let kinds: Vec<StoryKind> = stories.iter().map(|s| s.kind).collect();
        assert_eq!(kinds, vec![StoryKind::RowsMayDuplicate, StoryKind::Schema]);
        assert_eq!(stories[0].severity, Severity::Risk);
        assert!(
            stories[0].detail.contains("(revenue_date, user_id)"),
            "detail was: {}",
            stories[0].detail
        );
    }

    #[test]
    fn maintenance_lost_claims_cells_and_refusals() {
        let removed_cell = cell_verdict("{amount}", "raw.orders", Technique::KeyedFold, true);
        let model = model_with(vec![
            ChangeKind::MaintenanceLost,
            ChangeKind::CellRemoved {
                cell: "{amount}@NewData".to_string(),
                old: Box::new(removed_cell.clone()),
                source_survives: false,
            },
            ChangeKind::CellRemoved {
                cell: "{b}@NewData".to_string(),
                old: Box::new(removed_cell.clone()),
                source_survives: false,
            },
            ChangeKind::CellRemoved {
                cell: "{c}@NewData".to_string(),
                old: Box::new(removed_cell),
                source_survives: false,
            },
            ChangeKind::RefusalAdded(ProfileRefusal {
                code: None,
                text: "ScanUnbounded".to_string(),
            }),
        ]);
        let stories = narrate(&model);
        assert_eq!(stories.len(), 1);
        assert_eq!(stories[0].severity, Severity::Risk);
        assert!(
            stories[0].detail.ends_with(" Reason: ScanUnbounded."),
            "detail was: {}",
            stories[0].detail
        );
        assert_eq!(stories[0].changes.len(), 5);
    }

    #[test]
    fn row_key_widened_without_fan_out() {
        let old_grain = Grain {
            keys: vec![vec!["date".to_string(), "user".to_string()]],
        };
        let new_grain = Grain {
            keys: vec![vec![
                "date".to_string(),
                "user".to_string(),
                "name".to_string(),
            ]],
        };
        let model = model_with(vec![ChangeKind::Grain {
            subject: String::new(),
            old: old_grain,
            new: new_grain,
        }]);
        let stories = narrate(&model);
        assert_eq!(stories.len(), 1);
        assert_eq!(stories[0].kind, StoryKind::RowKey);
        assert_eq!(stories[0].severity, Severity::Risk);
        assert_eq!(stories[0].lead, "Row key widened");
    }

    #[test]
    fn technique_story_folds_state_downgrade_reason() {
        let model = model_with(vec![
            ChangeKind::CellTechnique {
                cell: "{amount}@NewData { source: \"raw.orders\" }".to_string(),
                group: "{amount}".to_string(),
                trigger_source: Some("raw.orders".to_string()),
                old: Technique::KeyedFold,
                new: Technique::DeleteInsert,
            },
            ChangeKind::StateDowngrade {
                cell: "{amount}@NewData { source: \"raw.orders\" }".to_string(),
                group: "{amount}".to_string(),
                trigger_source: Some("raw.orders".to_string()),
                old: None,
                new: Some(StateDowngrade {
                    original: Technique::KeyedFold,
                    missing: StateStructure::MergeLedger,
                    reason: "no running-aggregate state realised".to_string(),
                }),
            },
        ]);
        let stories = narrate(&model);
        assert_eq!(stories.len(), 1);
        assert_eq!(stories[0].kind, StoryKind::Technique);
        assert_eq!(stories[0].severity, Severity::Cost);
        assert!(
            stories[0]
                .detail
                .ends_with(" Reason: no running-aggregate state realised."),
            "detail was: {}",
            stories[0].detail
        );
        assert_eq!(stories[0].changes.len(), 2);
    }

    #[test]
    fn cell_technique_on_a_backfill_cell_reads_well_with_no_source() {
        // `trigger_source: None` (a `Backfill`/`ColumnAdded` cell has no
        // per-source address) must still produce a sentence, never one that
        // leaks the cell key's own `{group}@{trigger:?}` join text.
        let model = model_with(vec![ChangeKind::CellTechnique {
            cell: "{amount}@Backfill".to_string(),
            group: "{amount}".to_string(),
            trigger_source: None,
            old: Technique::DeleteInsert,
            new: Technique::KeyedFold,
        }]);
        let stories = narrate(&model);
        assert_eq!(stories.len(), 1);
        assert_eq!(stories[0].kind, StoryKind::Technique);
        assert_eq!(
            stories[0].detail,
            "Changes are applied to {amount} by KeyedFold instead of DeleteInsert."
        );
        // Never leaks the cell key's raw `Trigger` debug text (e.g. a
        // `ColumnAdded { columns: [...] }` trigger's own field name) into
        // the sentence.
        assert!(!stories[0].detail.contains("columns:"));
        assert!(!stories[0].detail.contains("Backfill"));
    }

    /// A removed cell whose source is still read by another, UNCHANGED
    /// sibling cell (so it never appears as a `cell_added`) grades a
    /// downgrade and must be claimed by the per-cell "Maintenance route
    /// lost" pass, not folded into a "Dependency removed" story
    /// (`docs/specs/property_diff.md` §"Direction" "cell_added"/
    /// "cell_removed" row).
    #[test]
    fn cell_removed_with_a_surviving_source_is_a_maintenance_route_lost_story() {
        let removed = cell_verdict("{amount}", "raw.orders", Technique::KeyedFold, true);
        let model = model_with(vec![ChangeKind::CellRemoved {
            cell: "{amount}@NewData { source: \"raw.orders\" }".to_string(),
            old: Box::new(removed),
            source_survives: true,
        }]);
        let stories = narrate(&model);
        assert_eq!(stories.len(), 1);
        assert_eq!(stories[0].kind, StoryKind::Dependency);
        assert_eq!(stories[0].lead, "Maintenance route lost");
        assert_eq!(
            stories[0].detail,
            "{amount} no longer has a maintenance route for changes from raw.orders."
        );
    }

    /// The mirror case: the source was dropped altogether (no surviving
    /// cell reads it) — `neutral`, folded into "Dependency removed".
    #[test]
    fn cell_removed_with_no_surviving_source_is_a_dependency_removed_story() {
        let removed = cell_verdict("{amount}", "raw.orders", Technique::KeyedFold, true);
        let model = model_with(vec![ChangeKind::CellRemoved {
            cell: "{amount}@NewData { source: \"raw.orders\" }".to_string(),
            old: Box::new(removed),
            source_survives: false,
        }]);
        let stories = narrate(&model);
        assert_eq!(stories.len(), 1);
        assert_eq!(stories[0].kind, StoryKind::Dependency);
        assert_eq!(stories[0].lead, "Dependency removed");
        assert_eq!(stories[0].detail, "No longer reads raw.orders.");
    }

    #[test]
    fn headline_and_lens_title() {
        fn story(kind: StoryKind, severity: Severity) -> Story {
            Story {
                kind,
                severity,
                subject: String::new(),
                lead: "x".to_string(),
                detail: "y".to_string(),
                changes: vec![],
            }
        }
        fn model(name: &str, stories: Vec<Story>) -> ModelDiff {
            ModelDiff {
                model: name.to_string(),
                cause: Cause {
                    kind: CauseKind::Edited,
                    of: vec![],
                    reason: None,
                },
                changes: vec![],
                stories,
            }
        }
        let report = DiffReport {
            baseline: BaselineInfo {
                r#ref: "main".to_string(),
                commit: "abc1234".to_string(),
                resolved_as: "merge_base".to_string(),
            },
            edited_files: vec![],
            summary: crate::analysis::diff::DiffSummary {
                downgrades: 3,
                upgrades: 1,
                neutral: 0,
                shifted_models: 4,
            },
            headline: String::new(),
            models: vec![
                model("a", vec![story(StoryKind::MaintenanceLost, Severity::Risk)]),
                model("b", vec![story(StoryKind::RowKey, Severity::Risk)]),
                model("c", vec![story(StoryKind::Reads, Severity::Cost)]),
                model("d", vec![story(StoryKind::Other, Severity::Improvement)]),
            ],
        };
        assert_eq!(
            headline(&report),
            "4 model(s) shifted · 1 lost incremental maintenance · 1 with correctness risks · 1 read more per run · 1 improved"
        );

        let baseline = &report.baseline;
        assert_eq!(lens_title(&report.models[1], baseline), "1 risk vs main");
        assert_eq!(
            lens_title(&report.models[2], baseline),
            "1 costlier vs main"
        );
        assert_eq!(lens_title(&report.models[3], baseline), "changed vs main");

        let mut cleared = report.clone();
        cleared.summary.downgrades = 0;
        assert!(headline(&cleared).ends_with(" · no downgrades"));
    }

    /// A small local stand-in constructor for `DerivedFd` — the type has no
    /// public constructor beyond its plain struct literal, kept next to its
    /// only use to avoid an extra import in the story-specific tests above.
    struct DerivedFdStub;
    impl DerivedFdStub {
        fn fd(key: &str, determines: &str) -> DerivedFd {
            DerivedFd {
                key: vec![key.to_string()],
                determines: determines.to_string(),
            }
        }
    }
}
