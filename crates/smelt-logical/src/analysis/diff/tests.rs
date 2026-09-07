use super::*;
use crate::contract::ContractPointView;
use crate::maintenance::{Corner, RowIdentity, Technique, Trigger};
use smelt_core::config::RetainDeparted;

fn set(row_identity: RowIdentityVerdict) -> PropertySet {
    PropertySet {
        columns: vec!["a".to_string()],
        grain: Grain::unkeyed(),
        functional_dependencies: Vec::new(),
        // Every mutation test below changes an existing column's
        // *value*, never its presence, so the base carries one
        // determinism/comparability/discriminant entry for "a" already;
        // the per-column diff loops below are keyed on the union but
        // need not synthesize a "missing" default.
        determinism: vec![crate::analysis::walk::ColumnDeterminism {
            output: "a".to_string(),
            level: Det::Clean,
        }],
        comparability: vec![crate::analysis::walk::ColumnComparability {
            output: "a".to_string(),
            comparability: Comp::Comparable,
        }],
        discriminants: vec![crate::analysis::walk::ColumnDiscriminant {
            output: "a".to_string(),
            discriminants: crate::analysis::discriminants::Discriminants {
                is_monoid: false,
                needs_inverse: false,
                decomposable: false,
                monotone: crate::analysis::discriminants::Monotone::None,
            },
        }],
        literal_columns: Vec::new(),
        has_set_op_barrier: false,
        has_fan_out_join: false,
        row_identity,
        source_bounds: BTreeMap::new(),
    }
}

fn base_set() -> PropertySet {
    set(RowIdentityVerdict {
        identity: RowIdentity::WholeRow,
        proven_mismatch: None,
    })
}

fn profile_from(properties: PropertySet) -> PropertyProfile {
    PropertyProfile {
        properties,
        cell_verdicts: Vec::new(),
        refusals: Vec::new(),
        probes: Vec::new(),
    }
}

fn cell(technique: Technique, group: &str, trigger_source: &str) -> CellVerdict {
    CellVerdict {
        group: group.to_string(),
        trigger: format!(
            "{:?}",
            Trigger::NewData {
                source: trigger_source.to_string()
            }
        ),
        corner: format!("{:?}", Corner::FoldDelta),
        technique,
        row_identity: RowIdentityVerdict {
            identity: RowIdentity::Key(vec!["id".to_string()]),
            proven_mismatch: None,
        },
        contract_point: ContractPointView::default(),
        state_downgrade: None,
        trigger_source: Some(trigger_source.to_string()),
        partition_local: true,
        locality_reason: None,
    }
}

/// A `cell()` whose maintenance is NOT partition-local — the case
/// `docs/specs/property_diff.md` §"Direction" grades `cell_added` a
/// downgrade on an already-maintained model.
fn non_local_cell(technique: Technique, group: &str, trigger_source: &str) -> CellVerdict {
    CellVerdict {
        partition_local: false,
        locality_reason: Some((
            trigger_source.to_string(),
            "no partition_column declared".to_string(),
        )),
        ..cell(technique, group, trigger_source)
    }
}

fn empty_graph() -> DiffGraph {
    DiffGraph::default()
}

// --- Direction table ---

#[test]
fn technique_downgrade_walks_the_ladder_down() {
    let old = cell(Technique::KeyedFold, "{a}", "orders");
    let new = cell(Technique::DeleteInsert, "{a}", "orders");
    let old_p = profile_from(base_set());
    let mut old_p = old_p;
    old_p.cell_verdicts = vec![old];
    let mut new_p = profile_from(base_set());
    new_p.cell_verdicts = vec![new];
    let changes = diff_profile(&old_p, &new_p);
    let technique_change = changes
        .iter()
        .find(|c| c.dimension == Dimension::CellTechnique)
        .expect("technique changed");
    assert_eq!(technique_change.direction, Direction::Downgrade);

    // Reverse direction.
    let changes_up = diff_profile(&new_p, &old_p);
    let up = changes_up
        .iter()
        .find(|c| c.dimension == Dimension::CellTechnique)
        .unwrap();
    assert_eq!(up.direction, Direction::Upgrade);
}

#[test]
fn ladder_is_total() {
    let ranks = [
        technique_rank(Technique::KeyedFold),
        technique_rank(Technique::ColumnScopedMerge),
        technique_rank(Technique::InPlaceUpdate),
        technique_rank(Technique::PerGroupRecompute),
        technique_rank(Technique::DeleteInsert),
    ];
    for w in ranks.windows(2) {
        assert!(w[0] > w[1], "ladder must be strictly descending: {ranks:?}");
    }
}

#[test]
fn cell_removed_from_maintained_model_is_a_downgrade() {
    let old = vec![
        cell(Technique::KeyedFold, "{a}", "orders"),
        cell(Technique::KeyedFold, "{b}", "orders"),
    ];
    let new = vec![cell(Technique::KeyedFold, "{a}", "orders")];
    let changes = diff_cell_verdicts(&old, &new);
    let removed = changes
        .iter()
        .find(|c| matches!(c, ChangeKind::CellRemoved { .. }))
        .unwrap();
    assert_eq!(removed.direction(), Direction::Downgrade);
}

/// A new dependency is a cost, not an upgrade
/// (`docs/specs/property_diff.md` §Design "A new dependency is a cost,
/// not an upgrade"): a non-partition-local cell added to an
/// already-maintained model reads its trigger source in full on every
/// run.
#[test]
fn cell_added_not_partition_local_is_a_downgrade() {
    let old = vec![cell(Technique::KeyedFold, "{a}", "orders")];
    let new = vec![
        cell(Technique::KeyedFold, "{a}", "orders"),
        non_local_cell(Technique::KeyedFold, "{b}", "devices"),
    ];
    let changes = diff_cell_verdicts(&old, &new);
    let added = changes
        .iter()
        .find(|c| matches!(c, ChangeKind::CellAdded { .. }))
        .unwrap();
    assert_eq!(added.direction(), Direction::Downgrade);
}

#[test]
fn cell_added_partition_local_is_neutral() {
    let old = vec![cell(Technique::KeyedFold, "{a}", "orders")];
    let new = vec![
        cell(Technique::KeyedFold, "{a}", "orders"),
        cell(Technique::KeyedFold, "{b}", "orders"),
    ];
    let changes = diff_cell_verdicts(&old, &new);
    let added = changes
        .iter()
        .find(|c| matches!(c, ChangeKind::CellAdded { .. }))
        .unwrap();
    assert_eq!(added.direction(), Direction::Neutral);

    // Zero cells -> one cell: `maintenance_gained` is the only upgrade
    // reported, never a per-cell `cell_added` upgrade.
    let mut old_p = profile_from(base_set());
    old_p.cell_verdicts = vec![];
    let mut new_p = profile_from(base_set());
    new_p.cell_verdicts = vec![cell(Technique::KeyedFold, "{a}", "orders")];
    let profile_changes = diff_profile(&old_p, &new_p);
    let upgrades: Vec<&Change> = profile_changes
        .iter()
        .filter(|c| c.direction == Direction::Upgrade)
        .collect();
    assert_eq!(
        upgrades.len(),
        1,
        "expected exactly one upgrade (maintenance_gained): {profile_changes:?}"
    );
    assert_eq!(upgrades[0].dimension, Dimension::MaintenanceGained);
}

/// `docs/specs/property_diff.md` §"Direction" "cell_added"/"cell_removed"
/// row: a removed cell is a downgrade only when another surviving cell
/// still reads the same trigger source.
#[test]
fn cell_removed_is_a_downgrade_only_when_its_source_survives() {
    let old = vec![
        cell(Technique::KeyedFold, "{a}", "orders"),
        cell(Technique::KeyedFold, "{b}", "orders"),
    ];
    let new = vec![cell(Technique::KeyedFold, "{a}", "orders")];
    let changes = diff_cell_verdicts(&old, &new);
    let removed = changes
        .iter()
        .find(|c| matches!(c, ChangeKind::CellRemoved { .. }))
        .unwrap();
    assert_eq!(
        removed.direction(),
        Direction::Downgrade,
        "source 'orders' still survives via {{a}}"
    );

    let old2 = vec![
        cell(Technique::KeyedFold, "{a}", "orders"),
        cell(Technique::KeyedFold, "{b}", "devices"),
    ];
    let new2 = vec![cell(Technique::KeyedFold, "{a}", "orders")];
    let changes2 = diff_cell_verdicts(&old2, &new2);
    let removed2 = changes2
        .iter()
        .find(|c| matches!(c, ChangeKind::CellRemoved { .. }))
        .unwrap();
    assert_eq!(
        removed2.direction(),
        Direction::Neutral,
        "source 'devices' no longer appears in any surviving cell"
    );
}

fn bounded(secs: u64) -> BoundResult {
    use crate::analysis::source_bounds::Seconds;
    BoundResult::Bounded {
        source_partition_col: "event_date".to_string(),
        before: Seconds(secs),
        after: Seconds(0),
    }
}

#[test]
fn source_bound_unbounding_is_a_downgrade() {
    let k = ChangeKind::SourceBound {
        source: "raw.orders".to_string(),
        old: bounded(60),
        new: BoundResult::Unbounded,
    };
    assert_eq!(k.direction(), Direction::Downgrade);
}

#[test]
fn source_bound_widening_is_a_downgrade() {
    let k = ChangeKind::SourceBound {
        source: "raw.orders".to_string(),
        old: bounded(60),
        new: bounded(120),
    };
    assert_eq!(k.direction(), Direction::Downgrade);
}

#[test]
fn source_bound_narrowing_is_an_upgrade() {
    let k = ChangeKind::SourceBound {
        source: "raw.orders".to_string(),
        old: bounded(120),
        new: bounded(60),
    };
    assert_eq!(k.direction(), Direction::Upgrade);
}

#[test]
fn source_bound_unbounded_to_not_derivable_is_neutral() {
    let k = ChangeKind::SourceBound {
        source: "raw.orders".to_string(),
        old: BoundResult::Unbounded,
        new: BoundResult::NotDerivable,
    };
    assert_eq!(k.direction(), Direction::Neutral);
    let reverse = ChangeKind::SourceBound {
        source: "raw.orders".to_string(),
        old: BoundResult::NotDerivable,
        new: BoundResult::Unbounded,
    };
    assert_eq!(reverse.direction(), Direction::Neutral);
}

/// `docs/specs/property_diff.md` §"Direction": a grain that widens (its
/// keys now cover a strict superset of the old column set — a weaker
/// uniqueness claim) is a downgrade; a grain that narrows (strict
/// subset — a stronger claim) is an upgrade; a keyed grain becoming
/// unkeyed stays a downgrade regardless; a different key of unrelated
/// columns is `neutral`, surfaced instead by the `row_key` story.
#[test]
fn grain_widening_is_a_downgrade_and_narrowing_an_upgrade() {
    let two = Grain {
        keys: vec![vec!["date".to_string(), "user".to_string()]],
    };
    let three = Grain {
        keys: vec![vec![
            "date".to_string(),
            "user".to_string(),
            "name".to_string(),
        ]],
    };
    let widened = ChangeKind::Grain {
        subject: String::new(),
        old: two.clone(),
        new: three.clone(),
    };
    assert_eq!(widened.direction(), Direction::Downgrade);

    let narrowed = ChangeKind::Grain {
        subject: String::new(),
        old: three,
        new: two,
    };
    assert_eq!(narrowed.direction(), Direction::Upgrade);

    // A different key of the same arity — neither a subset nor a
    // superset of the other.
    let ab = Grain {
        keys: vec![vec!["a".to_string(), "b".to_string()]],
    };
    let ac = Grain {
        keys: vec![vec!["a".to_string(), "c".to_string()]],
    };
    let different_key = ChangeKind::Grain {
        subject: String::new(),
        old: ab,
        new: ac,
    };
    assert_eq!(different_key.direction(), Direction::Neutral);

    // Keyed -> unkeyed stays a downgrade regardless of column-set logic.
    let full = Grain {
        keys: vec![vec!["id".to_string()]],
    };
    let empty = Grain::unkeyed();
    let unkeyed = ChangeKind::Grain {
        subject: String::new(),
        old: full,
        new: empty,
    };
    assert_eq!(unkeyed.direction(), Direction::Downgrade);
}

#[test]
fn grain_gained_is_an_upgrade() {
    let k = ChangeKind::Grain {
        subject: String::new(),
        old: Grain::unkeyed(),
        new: Grain {
            keys: vec![vec!["id".to_string()]],
        },
    };
    assert_eq!(k.direction(), Direction::Upgrade);
}

#[test]
fn row_identity_key_to_whole_row_is_a_downgrade() {
    let k = ChangeKind::RowIdentity {
        old: RowIdentityVerdict {
            identity: RowIdentity::Key(vec!["id".to_string()]),
            proven_mismatch: None,
        },
        new: RowIdentityVerdict {
            identity: RowIdentity::WholeRow,
            proven_mismatch: None,
        },
    };
    assert_eq!(k.direction(), Direction::Downgrade);
    let reverse = ChangeKind::RowIdentity {
        old: RowIdentityVerdict {
            identity: RowIdentity::WholeRow,
            proven_mismatch: None,
        },
        new: RowIdentityVerdict {
            identity: RowIdentity::Key(vec!["id".to_string()]),
            proven_mismatch: None,
        },
    };
    assert_eq!(reverse.direction(), Direction::Upgrade);
}

fn refusal(code: Option<&str>, text: &str) -> ProfileRefusal {
    ProfileRefusal {
        code: code.map(|c| c.to_string()),
        text: text.to_string(),
    }
}

#[test]
fn refusal_added_is_a_downgrade() {
    let k = ChangeKind::RefusalAdded(refusal(Some("MaintenanceScanUnbounded"), "boom"));
    assert_eq!(k.direction(), Direction::Downgrade);
}

#[test]
fn refusal_removed_is_an_upgrade() {
    let k = ChangeKind::RefusalRemoved(refusal(Some("MaintenanceScanUnbounded"), "boom"));
    assert_eq!(k.direction(), Direction::Upgrade);
}

#[test]
fn uncoded_refusals_match_on_text_not_on_a_shared_placeholder() {
    let old = vec![refusal(None, "reach not derivable: A")];
    let new = vec![refusal(None, "reach not derivable: B")];
    let changes = diff_refusals(&old, &new);
    assert_eq!(
        changes.len(),
        2,
        "expected one removed + one added: {changes:?}"
    );
    assert!(changes
        .iter()
        .any(|c| matches!(c, ChangeKind::RefusalRemoved(r) if r.text == "reach not derivable: A")));
    assert!(changes
        .iter()
        .any(|c| matches!(c, ChangeKind::RefusalAdded(r) if r.text == "reach not derivable: B")));

    // Unchanged None-coded refusal on both sides: no change at all.
    let same = vec![refusal(None, "reach not derivable: A")];
    assert!(diff_refusals(&same, &same).is_empty());
}

fn contract_view_default() -> ContractPointView {
    ContractPointView::default()
}

#[test]
fn contract_relaxation_added_is_a_downgrade() {
    let new = ContractPointView {
        frozen_horizon: Some("90 days".to_string()),
        frozen_horizon_seconds: Some(90 * 86400),
        ..Default::default()
    };
    let k = ChangeKind::ContractPoint {
        cell: "revenue@orders".to_string(),
        old: contract_view_default(),
        new,
    };
    assert_eq!(k.direction(), Direction::Downgrade);
}

#[test]
fn contract_horizon_widened_is_a_downgrade() {
    let old = ContractPointView {
        frozen_horizon: Some("90 days".to_string()),
        frozen_horizon_seconds: Some(90 * 86400),
        ..Default::default()
    };
    let new = ContractPointView {
        frozen_horizon: Some("180 days".to_string()),
        frozen_horizon_seconds: Some(180 * 86400),
        ..Default::default()
    };
    let k = ChangeKind::ContractPoint {
        cell: "revenue@orders".to_string(),
        old,
        new,
    };
    assert_eq!(k.direction(), Direction::Downgrade);
}

#[test]
fn contract_relaxation_removed_is_an_upgrade() {
    let old = ContractPointView {
        frozen_horizon: Some("90 days".to_string()),
        frozen_horizon_seconds: Some(90 * 86400),
        ..Default::default()
    };
    let k = ChangeKind::ContractPoint {
        cell: "revenue@orders".to_string(),
        old,
        new: contract_view_default(),
    };
    assert_eq!(k.direction(), Direction::Upgrade);
}

#[test]
fn retain_departed_appearing_is_a_downgrade() {
    let new = ContractPointView {
        retain_departed: Some("true".to_string()),
        ..Default::default()
    };
    let k = ChangeKind::ContractPoint {
        cell: "revenue@orders".to_string(),
        old: contract_view_default(),
        new,
    };
    assert_eq!(k.direction(), Direction::Downgrade);
}

#[test]
fn retain_departed_removed_is_an_upgrade() {
    let old = ContractPointView {
        retain_departed: Some("true".to_string()),
        ..Default::default()
    };
    let k = ChangeKind::ContractPoint {
        cell: "revenue@orders".to_string(),
        old,
        new: contract_view_default(),
    };
    assert_eq!(k.direction(), Direction::Upgrade);
}

#[test]
fn retain_departed_shape_change_is_neutral() {
    let _ = RetainDeparted::Bool(true); // sanity: type reachable in tests
    let old = ContractPointView {
        retain_departed: Some("true".to_string()),
        ..Default::default()
    };
    let new = ContractPointView {
        retain_departed: Some("tombstone: deleted_at".to_string()),
        ..Default::default()
    };
    let k = ChangeKind::ContractPoint {
        cell: "revenue@orders".to_string(),
        old,
        new,
    };
    assert_eq!(k.direction(), Direction::Neutral);
}

fn probe(fact: &str) -> ProfileProbe {
    ProfileProbe {
        fact: fact.to_string(),
        probe: "SomeProbe".to_string(),
        cell: "main.model (declared)".to_string(),
    }
}

#[test]
fn probe_removed_is_a_downgrade() {
    let k = ChangeKind::ProbeRemoved(probe("assert_monotonic"));
    assert_eq!(k.direction(), Direction::Downgrade);
}

#[test]
fn probe_added_is_an_upgrade() {
    let k = ChangeKind::ProbeAdded(probe("assert_monotonic"));
    assert_eq!(k.direction(), Direction::Upgrade);
}

#[test]
fn determinism_clean_to_run_is_a_downgrade() {
    let k = ChangeKind::Determinism {
        column: "c".to_string(),
        old: Some(Det::Clean),
        new: Some(Det::Run),
    };
    assert_eq!(k.direction(), Direction::Downgrade);
}

#[test]
fn determinism_run_to_row_is_a_downgrade() {
    let k = ChangeKind::Determinism {
        column: "c".to_string(),
        old: Some(Det::Run),
        new: Some(Det::Row),
    };
    assert_eq!(k.direction(), Direction::Downgrade);
}

#[test]
fn determinism_row_to_clean_is_an_upgrade() {
    let k = ChangeKind::Determinism {
        column: "c".to_string(),
        old: Some(Det::Row),
        new: Some(Det::Clean),
    };
    assert_eq!(k.direction(), Direction::Upgrade);
}

#[test]
fn column_added_removed_discriminant_and_corner_are_neutral() {
    assert_eq!(
        ChangeKind::ColumnAdded("c".to_string()).direction(),
        Direction::Neutral
    );
    assert_eq!(
        ChangeKind::ColumnRemoved("c".to_string()).direction(),
        Direction::Neutral
    );
    assert_eq!(
        ChangeKind::CellCorner {
            cell: "x".to_string(),
            group: "x".to_string(),
            trigger_source: None,
            old: "A".to_string(),
            new: "B".to_string()
        }
        .direction(),
        Direction::Neutral
    );
    use crate::analysis::discriminants::Discriminants;
    let d1 = Discriminants {
        is_monoid: true,
        needs_inverse: false,
        decomposable: false,
        monotone: crate::analysis::discriminants::Monotone::None,
    };
    let d2 = Discriminants {
        is_monoid: false,
        needs_inverse: false,
        decomposable: false,
        monotone: crate::analysis::discriminants::Monotone::None,
    };
    assert_eq!(
        ChangeKind::Discriminant {
            column: "c".to_string(),
            old: Some(d1),
            new: Some(d2)
        }
        .direction(),
        Direction::Neutral
    );
}

#[test]
fn comparability_loss_is_a_downgrade() {
    let k = ChangeKind::Comparability {
        column: "c".to_string(),
        old: Some(Comp::Comparable),
        new: Some(Comp::Incomparable),
    };
    assert_eq!(k.direction(), Direction::Downgrade);
    let reverse = ChangeKind::Comparability {
        column: "c".to_string(),
        old: Some(Comp::Incomparable),
        new: Some(Comp::Comparable),
    };
    assert_eq!(reverse.direction(), Direction::Upgrade);
}

#[test]
fn set_op_barrier_appearing_is_a_downgrade() {
    let k = ChangeKind::SetOpBarrier {
        old: false,
        new: true,
    };
    assert_eq!(k.direction(), Direction::Downgrade);
    let reverse = ChangeKind::SetOpBarrier {
        old: true,
        new: false,
    };
    assert_eq!(reverse.direction(), Direction::Upgrade);
}

#[test]
fn fan_out_join_appearing_is_a_downgrade() {
    let k = ChangeKind::FanOutJoin {
        old: false,
        new: true,
    };
    assert_eq!(k.direction(), Direction::Downgrade);
}

#[test]
fn fd_and_literal_changes_are_neutral() {
    let fd = DerivedFd {
        key: vec!["id".to_string()],
        determines: "amount".to_string(),
    };
    assert_eq!(
        ChangeKind::FdAdded(fd.clone()).direction(),
        Direction::Neutral
    );
    assert_eq!(ChangeKind::FdRemoved(fd).direction(), Direction::Neutral);
    assert_eq!(
        ChangeKind::LiteralColumn {
            column: "c".to_string(),
            old: None,
            new: Some("x".to_string())
        }
        .direction(),
        Direction::Neutral
    );
}

// --- Structural cases ---

#[test]
fn model_only_in_new_is_added_with_null_olds() {
    let old: BTreeMap<String, PropertyProfile> = BTreeMap::new();
    let mut new: BTreeMap<String, PropertyProfile> = BTreeMap::new();
    new.insert("m".to_string(), profile_from(base_set()));
    let diff = diff_profiles(&old, &new, &empty_graph());
    assert_eq!(diff.models.len(), 1);
    let m = &diff.models[0];
    assert!(matches!(m.cause.kind, CauseKind::Added));
    assert!(!m.changes.is_empty());
    for c in &m.changes {
        assert_eq!(c.old, None, "added model's changes must have old=null");
        // G6 (`docs/outcomes/20260905-property-diff` fix round 1): a
        // wholly new model's changes are never graded — the `cause`
        // already says `added`.
        assert_eq!(c.direction, Direction::Neutral);
    }
}

#[test]
fn model_only_in_old_is_removed() {
    let mut old: BTreeMap<String, PropertyProfile> = BTreeMap::new();
    old.insert("m".to_string(), profile_from(base_set()));
    let new: BTreeMap<String, PropertyProfile> = BTreeMap::new();
    let diff = diff_profiles(&old, &new, &empty_graph());
    assert_eq!(diff.models.len(), 1);
    let m = &diff.models[0];
    assert!(matches!(m.cause.kind, CauseKind::Removed));
    for c in &m.changes {
        assert_eq!(c.new, None, "removed model's changes must have new=null");
        assert_eq!(c.direction, Direction::Neutral);
    }
}

// --- Fix round 1 (G1, G5, G7) ---

#[test]
fn maintenance_lost_is_a_downgrade_and_gained_is_an_upgrade() {
    assert_eq!(
        ChangeKind::MaintenanceLost.direction(),
        Direction::Downgrade
    );
    assert_eq!(
        ChangeKind::MaintenanceGained.direction(),
        Direction::Upgrade
    );
    assert_eq!(
        ChangeKind::MaintenanceLost.dimension(),
        Dimension::MaintenanceLost
    );
    assert_eq!(
        ChangeKind::MaintenanceGained.dimension(),
        Dimension::MaintenanceGained
    );
}

/// R8 (`docs/outcomes/20260905-property-diff/phases/04-plan.md`
/// addendum): a cell's `state_downgrade` appearing is a downgrade,
/// disappearing is an upgrade, and a shape-only change (different
/// missing structure, presence unchanged) is neutral.
#[test]
fn state_downgrade_appearing_is_a_downgrade_disappearing_is_an_upgrade() {
    use crate::maintenance::availability::{StateDowngrade, StateStructure};

    let sd = StateDowngrade {
        original: Technique::KeyedFold,
        missing: StateStructure::FingerprintSidecar,
        reason: "no fingerprint sidecar on this target".to_string(),
    };
    assert_eq!(
        ChangeKind::StateDowngrade {
            cell: "c".to_string(),
            group: "c".to_string(),
            trigger_source: None,
            old: None,
            new: Some(sd.clone()),
        }
        .direction(),
        Direction::Downgrade
    );
    assert_eq!(
        ChangeKind::StateDowngrade {
            cell: "c".to_string(),
            group: "c".to_string(),
            trigger_source: None,
            old: Some(sd.clone()),
            new: None,
        }
        .direction(),
        Direction::Upgrade
    );
    let sd2 = StateDowngrade {
        missing: StateStructure::MergeLedger,
        ..sd.clone()
    };
    assert_eq!(
        ChangeKind::StateDowngrade {
            cell: "c".to_string(),
            group: "c".to_string(),
            trigger_source: None,
            old: Some(sd),
            new: Some(sd2),
        }
        .direction(),
        Direction::Neutral
    );
    assert_eq!(
        ChangeKind::StateDowngrade {
            cell: "c".to_string(),
            group: "c".to_string(),
            trigger_source: None,
            old: None,
            new: None,
        }
        .dimension(),
        Dimension::StateDowngrade
    );
}

/// A matched cell whose `state_downgrade` differs must surface as a
/// `state_downgrade` change even when technique/corner/row-identity/
/// contract point are unchanged — the case a plain `cell_technique`
/// diff would miss if the ideal and degraded techniques happened to
/// coincide on both sides for an unrelated reason.
#[test]
fn diff_cell_verdicts_surfaces_a_state_downgrade_on_an_otherwise_unchanged_cell() {
    use crate::maintenance::availability::{StateDowngrade, StateStructure};

    let mut old_cell = cell(Technique::DeleteInsert, "{a}", "orders");
    let mut new_cell = old_cell.clone();
    new_cell.state_downgrade = Some(StateDowngrade {
        original: Technique::KeyedFold,
        missing: StateStructure::FingerprintSidecar,
        reason: "no fingerprint sidecar on this target".to_string(),
    });
    old_cell.state_downgrade = None;

    let changes = diff_cell_verdicts(&[old_cell], &[new_cell]);
    assert!(
        changes.iter().any(|c| matches!(
            c,
            ChangeKind::StateDowngrade {
                old: None,
                new: Some(_),
                ..
            }
        )),
        "expected a state_downgrade change, got {changes:?}"
    );
}

/// G1: `refresh: incremental` -> `refresh: full` with byte-identical SQL
/// yields empty cells and empty refusals on the new side — before this
/// fix, N `cell_removed` changes all graded `Neutral` and nothing
/// downgraded. `diff_profile` must emit exactly one `maintenance_lost`
/// change, graded `Downgrade`, alongside the (neutral) per-cell removals.
#[test]
fn losing_maintenance_entirely_surfaces_as_one_downgrade() {
    let mut old_profile = profile_from(base_set());
    old_profile.cell_verdicts = vec![
        cell(Technique::KeyedFold, "{a}", "orders"),
        cell(Technique::KeyedFold, "{b}", "orders"),
    ];
    let mut new_profile = old_profile.clone();
    new_profile.cell_verdicts = Vec::new();

    let changes = diff_profile(&old_profile, &new_profile);
    let lost: Vec<_> = changes
        .iter()
        .filter(|c| c.dimension == Dimension::MaintenanceLost)
        .collect();
    assert_eq!(
        lost.len(),
        1,
        "expected exactly one maintenance_lost change: {changes:?}"
    );
    assert_eq!(lost[0].direction, Direction::Downgrade);
    assert!(
        changes.iter().any(|c| c.direction == Direction::Downgrade),
        "the model must show at least one downgrade when it stops being maintained"
    );
    // The per-cell removals themselves stay neutral — the event is
    // named once, not N times.
    for c in changes
        .iter()
        .filter(|c| c.dimension == Dimension::CellRemoved)
    {
        assert_eq!(c.direction, Direction::Neutral);
    }
}

#[test]
fn grain_composite_key_column_dropped_is_an_upgrade() {
    // Key(["id", "region"]) -> Key(["id"]): the composite narrowed to a
    // strict subset of the old column set — a *stronger* uniqueness
    // claim ("proving one row per (id) implies one row per (id,
    // region)"), per spec §Direction's paragraph on grain widening.
    let k = ChangeKind::Grain {
        subject: String::new(),
        old: Grain {
            keys: vec![vec!["id".to_string(), "region".to_string()]],
        },
        new: Grain {
            keys: vec![vec!["id".to_string()]],
        },
    };
    assert_eq!(k.direction(), Direction::Upgrade);

    // Symmetric: widening the composite by adding a column is a
    // downgrade.
    let reverse = ChangeKind::Grain {
        subject: String::new(),
        old: Grain {
            keys: vec![vec!["id".to_string()]],
        },
        new: Grain {
            keys: vec![vec!["id".to_string(), "region".to_string()]],
        },
    };
    assert_eq!(reverse.direction(), Direction::Downgrade);
}

#[test]
fn fd_multiplicity_is_respected() {
    // Two copies of the same FD on the old side, one on the new side:
    // a plain `.contains` membership check sees the FD "still present"
    // and misses the removal of the duplicate.
    let fd = DerivedFd {
        key: vec!["id".to_string()],
        determines: "amount".to_string(),
    };
    let old = vec![fd.clone(), fd.clone()];
    let new = vec![fd.clone()];
    assert_eq!(multiset_excess(&old, &new), vec![fd.clone()]);
    assert!(multiset_excess(&new, &old).is_empty());
}

#[test]
fn identical_profiles_are_unshifted() {
    let mut old: BTreeMap<String, PropertyProfile> = BTreeMap::new();
    old.insert("m".to_string(), profile_from(base_set()));
    let new = old.clone();
    let diff = diff_profiles(&old, &new, &empty_graph());
    assert!(diff.models.is_empty());
    assert_eq!(diff.summary.shifted_models, 0);
    assert_eq!(diff.summary.downgrades, 0);
    assert_eq!(diff.summary.upgrades, 0);
    assert_eq!(diff.summary.neutral, 0);
}

#[test]
fn every_profile_difference_produces_at_least_one_change() {
    let base = base_set();

    let mut grain_changed = base.clone();
    grain_changed.grain = Grain {
        keys: vec![vec!["id".to_string()]],
    };
    let mut fd_changed = base.clone();
    fd_changed.functional_dependencies = vec![DerivedFd {
        key: vec![],
        determines: "c".to_string(),
    }];
    let mut det_changed = base.clone();
    det_changed.columns = vec!["a".to_string()];
    det_changed.determinism = vec![crate::analysis::walk::ColumnDeterminism {
        output: "a".to_string(),
        level: Det::Run,
    }];
    let mut comp_changed = base.clone();
    comp_changed.columns = vec!["a".to_string()];
    comp_changed.comparability = vec![crate::analysis::walk::ColumnComparability {
        output: "a".to_string(),
        comparability: Comp::Incomparable,
    }];
    let mut disc_changed = base.clone();
    disc_changed.columns = vec!["a".to_string()];
    disc_changed.discriminants = vec![crate::analysis::walk::ColumnDiscriminant {
        output: "a".to_string(),
        discriminants: crate::analysis::discriminants::Discriminants {
            is_monoid: true,
            needs_inverse: false,
            decomposable: false,
            monotone: crate::analysis::discriminants::Monotone::None,
        },
    }];
    let mut lit_changed = base.clone();
    lit_changed.literal_columns = vec![("a".to_string(), "1".to_string())];
    let mut set_op_changed = base.clone();
    set_op_changed.has_set_op_barrier = true;
    let mut fan_out_changed = base.clone();
    fan_out_changed.has_fan_out_join = true;
    let mut row_identity_changed = base.clone();
    row_identity_changed.row_identity = RowIdentityVerdict {
        identity: RowIdentity::Key(vec!["id".to_string()]),
        proven_mismatch: None,
    };
    let mut source_bound_changed = base.clone();
    source_bound_changed
        .source_bounds
        .insert("raw.orders".to_string(), BoundResult::Unbounded);
    let mut columns_changed = base.clone();
    columns_changed.columns.push("extra".to_string());

    // G3/G4 (`docs/outcomes/20260905-property-diff` fix round 1): remove
    // the per-column entry entirely while `columns` stays unchanged —
    // the presence-asymmetry case the value-only mutations above cannot
    // exercise. `literal_columns` is exempted: it already diffs the
    // union of keys (`:867`), so removing its one entry is covered by
    // `lit_changed` above via a value-level round trip already.
    let mut det_removed = base.clone();
    det_removed.determinism = Vec::new();
    let mut comp_removed = base.clone();
    comp_removed.comparability = Vec::new();
    let mut disc_removed = base.clone();
    disc_removed.discriminants = Vec::new();

    let mutations = [
        grain_changed,
        fd_changed,
        det_changed,
        comp_changed,
        disc_changed,
        lit_changed,
        set_op_changed,
        fan_out_changed,
        row_identity_changed,
        source_bound_changed,
        columns_changed,
        det_removed,
        comp_removed,
        disc_removed,
    ];

    for m in mutations {
        let changed = m != base;
        let kinds = diff_property_set(&base, &m);
        assert_eq!(
            changed,
            !kinds.is_empty(),
            "PropertySet difference must produce a change (or the profile must actually be equal): {m:?} vs {base:?}"
        );
    }
}

/// G2/G4 (`docs/outcomes/20260905-property-diff` fix round 1): the
/// PropertySet-level test above cannot see `cell_verdicts`/`refusals`/
/// `probes` at all — this covers the rest of [`PropertyProfile`]. Before
/// the G2/G1 fixes this test caught: a matched probe whose `probe` field
/// changed produced zero changes (`diff_probes` matched on `(fact,
/// cell)` only and never compared the third field), and a model whose
/// cell list went from non-empty to empty produced only `Neutral`
/// `cell_removed` changes with no dimension naming the event itself.
#[test]
fn every_profile_field_difference_produces_at_least_one_change() {
    let mut base = profile_from(base_set());
    base.cell_verdicts = vec![cell(Technique::KeyedFold, "{a}", "orders")];
    base.refusals = vec![refusal(Some("MaintenanceScanUnbounded"), "boom")];
    base.probes = vec![probe("assert_monotonic")];

    let mut probe_fact_changed = base.clone();
    probe_fact_changed.probes = vec![ProfileProbe {
        fact: "assert_other".to_string(),
        ..probe("assert_monotonic")
    }];
    let mut probe_diagnostic_changed = base.clone();
    probe_diagnostic_changed.probes = vec![ProfileProbe {
        probe: "SomeOtherProbe".to_string(),
        ..probe("assert_monotonic")
    }];
    let mut probe_cell_changed = base.clone();
    probe_cell_changed.probes = vec![ProfileProbe {
        cell: "main.other_model (declared)".to_string(),
        ..probe("assert_monotonic")
    }];
    let mut refusal_code_changed = base.clone();
    refusal_code_changed.refusals = vec![refusal(Some("MaintenanceUnsupportedGrain"), "boom")];
    let mut refusal_text_changed = base.clone();
    refusal_text_changed.refusals =
        vec![refusal(Some("MaintenanceScanUnbounded"), "different text")];
    let mut maintenance_lost = base.clone();
    maintenance_lost.cell_verdicts = Vec::new();
    let mut all_empty = profile_from(base_set());
    let mut maintenance_gained = all_empty.clone();
    maintenance_gained.cell_verdicts = vec![cell(Technique::KeyedFold, "{a}", "orders")];
    all_empty.refusals = Vec::new();
    all_empty.probes = Vec::new();

    let mutations = [
        probe_fact_changed,
        probe_diagnostic_changed,
        probe_cell_changed,
        refusal_code_changed,
        refusal_text_changed,
        maintenance_lost,
    ];
    for m in mutations {
        let changed = m != base;
        let changes = diff_profile(&base, &m);
        assert_eq!(
            changed,
            !changes.is_empty(),
            "PropertyProfile difference must produce a change: {m:?} vs {base:?}"
        );
    }

    // Symmetric gained-maintenance case, diffed the other way round.
    assert_ne!(all_empty, maintenance_gained);
    assert!(
        !diff_profile(&all_empty, &maintenance_gained).is_empty(),
        "a model gaining maintenance (empty cells -> non-empty) must produce a change"
    );
}

#[test]
fn renamed_column_is_removal_plus_addition() {
    let mut base = base_set();
    base.columns = vec!["a".to_string()];
    let mut renamed = base_set();
    renamed.columns = vec!["b".to_string()];
    let kinds = diff_property_set(&base, &renamed);
    assert!(kinds
        .iter()
        .any(|k| matches!(k, ChangeKind::ColumnRemoved(c) if c == "a")));
    assert!(kinds
        .iter()
        .any(|k| matches!(k, ChangeKind::ColumnAdded(c) if c == "b")));
}

#[test]
fn cells_match_on_group_and_trigger() {
    let old = vec![cell(Technique::KeyedFold, "{a}", "orders")];
    let new = vec![cell(Technique::KeyedFold, "{a}", "returns")];
    let changes = diff_cell_verdicts(&old, &new);
    assert!(changes
        .iter()
        .any(|c| matches!(c, ChangeKind::CellRemoved { .. })));
    assert!(changes
        .iter()
        .any(|c| matches!(c, ChangeKind::CellAdded { .. })));
    assert!(!changes
        .iter()
        .any(|c| matches!(c, ChangeKind::CellTechnique { .. })));
}

// --- Attribution ---

fn graph_with(upstream: &[(&str, &[&str])], edited: &[&str]) -> DiffGraph {
    let mut g = DiffGraph::default();
    for (n, ups) in upstream {
        g.upstream
            .insert(n.to_string(), ups.iter().map(|s| s.to_string()).collect());
    }
    g.edited = edited.iter().map(|s| s.to_string()).collect();
    g
}

#[test]
fn own_file_edited_is_cause_edited() {
    let g = graph_with(&[("a", &[])], &["a"]);
    let cause = g.attribute("a");
    assert!(matches!(cause.kind, CauseKind::Edited));
}

#[test]
fn downstream_names_nearest_edited_ancestor() {
    // src -> a(edited) -> b
    let g = graph_with(&[("b", &["a"]), ("a", &["src"])], &["a"]);
    let cause = g.attribute("b");
    assert!(matches!(cause.kind, CauseKind::Downstream));
    assert_eq!(cause.of, vec!["a".to_string()]);
}

#[test]
fn attribution_stops_at_the_first_edited_node() {
    // a(edited) -> b(edited) -> c
    let g = graph_with(&[("c", &["b"]), ("b", &["a"])], &["a", "b"]);
    let cause = g.attribute("c");
    assert_eq!(cause.of, vec!["b".to_string()]);
}

#[test]
fn edited_source_is_a_valid_ancestor() {
    let g = graph_with(&[("m", &["raw.orders"])], &["raw.orders"]);
    let cause = g.attribute("m");
    assert_eq!(cause.of, vec!["raw.orders".to_string()]);
}

#[test]
fn two_edited_ancestors_are_both_listed_sorted() {
    let g = graph_with(&[("c", &["a", "b"])], &["a", "b"]);
    let cause = g.attribute("c");
    assert_eq!(cause.of, vec!["a".to_string(), "b".to_string()]);
}

#[test]
fn no_edited_ancestor_yields_project_config_cause() {
    let mut g = graph_with(&[("m", &[])], &[]);
    g.project_config_changed = true;
    let cause = g.attribute("m");
    assert!(matches!(cause.kind, CauseKind::Downstream));
    assert!(cause.of.is_empty());
    assert_eq!(
        cause.reason,
        Some("project configuration changed".to_string())
    );
}

// --- Serialization / summary ---

#[test]
fn change_json_matches_the_spec_schema() {
    let k = ChangeKind::RefusalAdded(refusal(Some("MaintenanceScanUnbounded"), "boom"));
    let change = Change::from_kind(k);
    let v = serde_json::to_value(&change).unwrap();
    let obj = v.as_object().unwrap();
    let mut keys: Vec<&str> = obj.keys().map(|s| s.as_str()).collect();
    keys.sort();
    assert_eq!(
        keys,
        vec!["dimension", "direction", "new", "old", "reason", "subject"]
    );
    assert_eq!(v["dimension"], "refusal_added");
}

#[test]
fn summary_counts_directions() {
    let mut old: BTreeMap<String, PropertyProfile> = BTreeMap::new();
    let mut new: BTreeMap<String, PropertyProfile> = BTreeMap::new();
    let mut old_profile = profile_from(base_set());
    old_profile.cell_verdicts = vec![cell(Technique::KeyedFold, "{a}", "orders")];
    let mut new_profile = old_profile.clone();
    new_profile.cell_verdicts = vec![cell(Technique::DeleteInsert, "{a}", "orders")];
    old.insert("m".to_string(), old_profile);
    new.insert("m".to_string(), new_profile);
    let diff = diff_profiles(&old, &new, &empty_graph());
    assert_eq!(diff.summary.downgrades, 1);
    assert_eq!(diff.summary.shifted_models, 1);
}

fn model_diff_stub(model: &str, kind: CauseKind) -> ModelDiff {
    ModelDiff {
        model: model.to_string(),
        cause: Cause {
            kind,
            of: vec![],
            reason: None,
        },
        changes: vec![],
        stories: vec![],
    }
}

#[test]
fn apply_failure_reasons_added_uses_baseline_failures() {
    // "added" = present in the working tree, absent from the
    // baseline — a derivation failure explaining that absence lives on
    // the BASELINE side.
    let mut diff = PropertyDiff {
        models: vec![model_diff_stub("m", CauseKind::Added)],
        summary: DiffSummary::default(),
    };
    let mut base_failures = BTreeMap::new();
    base_failures.insert("m".to_string(), "parse error: bad SQL".to_string());
    let work_failures = BTreeMap::new();

    apply_failure_reasons(&mut diff, &base_failures, &work_failures);

    assert_eq!(
        diff.models[0].cause.reason.as_deref(),
        Some("parse error: bad SQL")
    );
}

#[test]
fn apply_failure_reasons_removed_uses_working_tree_failures() {
    // "removed" = present in the baseline, absent from the working
    // tree — a derivation failure explaining that absence lives on the
    // WORKING TREE side (fix round 1, Q2: this was backwards before —
    // a working-tree derivation failure was looked up in
    // `base_failures`, which is never populated for it, so the reason
    // silently never applied).
    let mut diff = PropertyDiff {
        models: vec![model_diff_stub("m", CauseKind::Removed)],
        summary: DiffSummary::default(),
    };
    let base_failures = BTreeMap::new();
    let mut work_failures = BTreeMap::new();
    work_failures.insert(
        "m".to_string(),
        "working-tree derivation failed".to_string(),
    );

    apply_failure_reasons(&mut diff, &base_failures, &work_failures);

    assert_eq!(
        diff.models[0].cause.reason.as_deref(),
        Some("working-tree derivation failed")
    );
}

#[test]
fn apply_failure_reasons_leaves_a_genuinely_added_model_with_no_reason() {
    let mut diff = PropertyDiff {
        models: vec![model_diff_stub("m", CauseKind::Added)],
        summary: DiffSummary::default(),
    };
    apply_failure_reasons(&mut diff, &BTreeMap::new(), &BTreeMap::new());
    assert!(diff.models[0].cause.reason.is_none());
}

#[test]
fn narrow_to_retains_only_selected_models_and_recomputes_the_summary() {
    let mut changed = model_diff_stub("kept", CauseKind::Edited);
    changed
        .changes
        .push(Change::from_kind(ChangeKind::MaintenanceLost));
    let dropped = model_diff_stub("dropped", CauseKind::Edited);
    let mut report = DiffReport {
        baseline: BaselineInfo {
            r#ref: "main".to_string(),
            commit: "abc".to_string(),
            resolved_as: "merge_base".to_string(),
        },
        edited_files: vec![],
        summary: DiffSummary {
            downgrades: 1,
            upgrades: 0,
            neutral: 0,
            shifted_models: 2,
        },
        headline: String::new(),
        models: vec![changed, dropped],
    };
    let selected: BTreeSet<String> = ["kept".to_string()].into_iter().collect();
    report.narrow_to(&selected);
    assert_eq!(report.models.len(), 1);
    assert_eq!(report.models[0].model, "kept");
    assert_eq!(report.summary.shifted_models, 1);
    assert_eq!(report.summary.downgrades, 1);
}
