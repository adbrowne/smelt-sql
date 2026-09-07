use crate::support::build_report_for;
use smelt_cli::build_maintenance_plan_report;

fn stage_repair_project(
    recipe: &smelt_maintenance_testkit::recipe::RepairRecipe,
    tmp: &tempfile::TempDir,
) -> std::path::PathBuf {
    let project_dir = tmp.path().join("project");
    let db_path = tmp.path().join("db.duckdb");
    std::fs::create_dir_all(&project_dir).expect("create project dir");
    smelt_maintenance_testkit::render::stage_repair(recipe, &project_dir, &db_path)
        .expect("stage_repair");
    project_dir
}

#[test]
fn explain_renders_repair_cell_key_slice_and_read_bound() {
    use smelt_maintenance_testkit::recipe::{KeyedCombiner, RepairRecipe, RepairWriteMode};

    let tmp = tempfile::tempdir().expect("tempdir");
    let recipe = RepairRecipe::new(
        KeyedCombiner::Idempotent,
        RepairWriteMode::TargetedDeleteInsert,
    );
    let project_dir = stage_repair_project(&recipe, &tmp);

    let report = build_report_for(&project_dir, &recipe.model_name)
        .expect("repair recipe has a maintenance plan");

    assert!(
        report.contains("technique: PerGroupRecompute"),
        "expected a PerGroupRecompute cell: {report}"
    );
    assert!(
        report.contains("repair key slice: customer_id (sound over-approximation)"),
        "expected the affected-key slice, labelled a sound over-approximation: {report}"
    );
    assert!(
        report.contains("repair read bound: source=repair_orders column=order_date"),
        "expected the bounded per-group read slice: {report}"
    );
}

#[test]
fn explain_renders_repair_discovery_posture() {
    use smelt_maintenance_testkit::recipe::{KeyedCombiner, RepairRecipe, RepairWriteMode};

    let tmp = tempfile::tempdir().expect("tempdir");
    let recipe = RepairRecipe::new(
        KeyedCombiner::Idempotent,
        RepairWriteMode::TargetedDeleteInsert,
    );
    let project_dir = stage_repair_project(&recipe, &tmp);

    let report = build_report_for(&project_dir, &recipe.model_name)
        .expect("repair recipe has a maintenance plan");

    assert!(
        report.contains(
            "affected-key discovery: group-grain fingerprint-sidecar diff (mutable_snapshot, \
             obligation 7)"
        ),
        "expected the group-grain sidecar diff discovery mechanism for a mutable_snapshot \
         source: {report}"
    );
}

#[test]
fn explain_renders_diff_patch_write_mechanism_and_delete_leg() {
    use smelt_maintenance_testkit::recipe::{KeyedCombiner, RepairRecipe, RepairWriteMode};

    let tmp = tempfile::tempdir().expect("tempdir");
    let recipe = RepairRecipe::new(KeyedCombiner::Idempotent, RepairWriteMode::DiffPatch);
    let project_dir = stage_repair_project(&recipe, &tmp);

    let report = build_report_for(&project_dir, &recipe.model_name)
        .expect("repair recipe has a maintenance plan");

    assert!(
        report.contains("write mechanism: diff_patch"),
        "expected the resolved diff_patch write mechanism: {report}"
    );
    assert!(
        report.contains("diff_patch delete leg: complete"),
        "expected a complete delete leg — PerGroupRecompute's own key-temporal-locality \
         premise discharges it: {report}"
    );
}

#[test]
fn explain_non_repair_cell_prints_no_repair_stanza() {
    use std::collections::BTreeSet;

    use smelt_cli::explain::RelationContractView;
    use smelt_db::queries::maintenance::MaintenancePlanResult;
    use smelt_logical::maintenance::{
        ColumnGroup, Corner, MaintenancePlan, PartitionLocal, PlanCell, RowIdentity,
        RowIdentityVerdict, Technique, Trigger,
    };

    let cell = PlanCell {
        group: "{max_val}".to_string(),
        trigger: Trigger::UpstreamMutation {
            source: "orders".to_string(),
        },
        corner: Corner::ColumnMerge,
        technique: Technique::KeyedFold,
        partition_local: PartitionLocal::Yes,
        scans: vec![],
        ledger_catch_up: false,
        row_identity: RowIdentityVerdict {
            identity: RowIdentity::Key(vec!["customer_id".to_string()]),
            proven_mismatch: None,
        },
        skeleton_source_closure: None,
        fingerprint_projections: Default::default(),
        key_scope: None,
        state_downgrade: None,
    };
    let result = MaintenancePlanResult {
        plan: MaintenancePlan {
            cells: vec![cell],
            refusals: vec![],
            key_locality: None,
        },
        column_groups: vec![ColumnGroup {
            columns: vec!["max_val".to_string()],
            mutation_sensitivity: Default::default(),
            membership_sensitivity: BTreeSet::new(),
        }],
        degenerate: vec![],
        state_columns: vec![],
        execution_postures: None,
        is_snapshot_reconcile: None,
        comparability: vec![],
        succession_advisories: vec![],
        succession_recipe: None,
    };
    let properties = smelt_logical::analysis::profile::PropertySet::derive(
        "non_repair_fixture",
        "SELECT 1 AS max_val",
        &[],
        &smelt_logical::analysis::source_bounds::BoundContext::default(),
    )
    .expect("PropertySet::derive");
    let contract_points: Vec<smelt_logical::contract::ContractPointView> = result
        .plan
        .cells
        .iter()
        .map(|_| smelt_logical::contract::effective_contract(None, "", &[]).into())
        .collect();
    let profile = smelt_logical::analysis::profile::PropertyProfile::assemble(
        properties,
        &result.plan.cells,
        &contract_points,
        &result.plan.refusals,
        &[],
    );
    let report = build_maintenance_plan_report(
        "non_repair_fixture",
        &result,
        &RelationContractView::from_facts(None, None),
        &[],
        &[],
        None,
        None,
        &[],
        &[],
        smelt_core::config::ProbeCadence::PerRun,
        &[],
        None,
        None,
        &profile,
        None,
    )
    .expect("build_maintenance_plan_report");

    assert!(
        report.contains("technique: KeyedFold"),
        "expected the KeyedFold cell to still print: {report}"
    );
    assert!(
        !report.contains("repair key slice"),
        "a non-repair cell must print no repair stanza: {report}"
    );
    assert!(
        !report.contains("repair read bound"),
        "a non-repair cell must print no repair stanza: {report}"
    );
    assert!(
        !report.contains("affected-key discovery"),
        "a non-repair cell must print no repair stanza: {report}"
    );
    assert!(
        !report.contains("write mechanism: diff_patch"),
        "a non-repair cell must print no repair stanza: {report}"
    );
}

// =============================================================================
// Output-delta edge typing (`docs/outcomes/20260809-output-delta-typing/
// outcome.md` phase 10; `docs/specs/incremental_models.md` §Surface "CLI"):
// each inbound edge's rendered `delta type:` row and its degradation
// reason, plus the key-addressed repair cell's upstream-sidecar discovery
// line.
// =============================================================================
