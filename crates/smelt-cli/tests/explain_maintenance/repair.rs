use std::process::Command;

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
        fold_grade: None,
    };
    let result = MaintenancePlanResult {
        plan: MaintenancePlan {
            cells: vec![cell],
            refusals: vec![],
            key_locality: None,
            retention_downgrades: Vec::new(),
            retention_reaches: Vec::new(),
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

// =============================================================================
// Phase 6c (`docs/outcomes/20260913-trino-incremental/phases/06c-plan.md`):
// a repair-admitted `PerGroupRecompute` cell's own state downgrade —
// criterion 3's named, explain-visible downgrade when the fingerprint
// sidecar has no realisation.
// =============================================================================

const REPAIR_SIDECAR_SMELT_YML: &str = "name: repair_sidecar_downgrade_fixture\n\
    version: 1\n\
    paths:\n  - models\n\
    targets:\n  dev:\n    type: duckdb\n    schema: main\n\
    default_materialization: view\n\
    state:\n  warehouse_tables: none\n";

const REPAIR_SIDECAR_ORDERS_SOURCE: &str = "description: orders\n\
    columns:\n\
    - name: order_id\n  type: INTEGER\n\
    - name: customer_id\n  type: INTEGER\n\
    - name: amount\n  type: DECIMAL(10,2)\n\
    - name: order_date\n  type: TIMESTAMP\n\
    timeseries:\n  event_time_column: order_date\n  partition_column: order_date\n  \
    granularity: day\n\
    unique_key: [order_id]\n\
    mutation_profile:\n  kind: mutable_snapshot\n";

const REPAIR_SIDECAR_MODEL_SQL: &str = "---\n\
     materialization: table\n\
     refresh: incremental\n\
     grain: key\n\
     unique_key: customer_id\n\
     ---\n\
     SELECT customer_id, MAX(amount) AS max_amount \
     FROM smelt.sources.orders \
     WHERE order_date BETWEEN TIMESTAMP '2025-01-12' - INTERVAL '1 day' AND TIMESTAMP \
     '2025-01-12' \
     GROUP BY customer_id\n";

/// A repair-admitted `PerGroupRecompute` cell (`MAX`, a non-invertible
/// combiner, over a `mutable_snapshot` source with a bounded Form B band)
/// downgrades to `DeleteInsert` on a project with `state.warehouse_tables:
/// none` — the fingerprint sidecar its group-grain affected-key discovery
/// always needs (phase 6c) has no realisation there. `smelt explain --json`
/// must render the downgrade naming the missing structure and the
/// replacement technique.
#[test]
fn explain_shows_the_repair_sidecar_downgrade() {
    let tmp = tempfile::TempDir::new().expect("create tempdir");
    std::fs::write(tmp.path().join("smelt.yml"), REPAIR_SIDECAR_SMELT_YML).unwrap();
    std::fs::create_dir_all(tmp.path().join("models/sources")).unwrap();
    std::fs::write(
        tmp.path().join("models/sources/orders.yml"),
        REPAIR_SIDECAR_ORDERS_SOURCE,
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("models/customer_max_amount.sql"),
        REPAIR_SIDECAR_MODEL_SQL,
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_smelt"))
        .arg("explain")
        .arg("customer_max_amount")
        .arg("--json")
        .arg("--project-dir")
        .arg(tmp.path())
        .output()
        .expect("spawn smelt explain customer_max_amount --json");

    assert!(
        output.status.success(),
        "smelt explain customer_max_amount --json failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("explain --json output must parse: {e}\n{stdout}"));

    let cells = json["cells"].as_array().expect("cells array");
    let downgraded = cells
        .iter()
        .find(|c| c.get("state_downgrade").is_some())
        .unwrap_or_else(|| panic!("expected a cell carrying state_downgrade: {stdout}"));
    assert_eq!(downgraded["technique"], "DeleteInsert");
    let downgrade = &downgraded["state_downgrade"];
    assert_eq!(downgrade["original"], "PerGroupRecompute");
    assert!(downgrade["missing"]
        .as_str()
        .unwrap()
        .contains("fingerprint sidecar"));
}
