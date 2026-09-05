//! MP7 (`docs/plans/20260707-maintenance-plan-impl.md`): `smelt explain
//! <model>` — the maintenance-plan report (`incremental_models.md` §Surface
//! "CLI"). Covers the pure report-string builder directly (fast) plus one
//! full CLI-argument-parsing path (spawns the real `smelt` binary) so the
//! wiring itself is exercised.

use std::path::Path;
use std::process::Command;

use smelt_cli::argument_resolution::{compute_scope, resolve_argument};
use smelt_cli::{
    build_maintenance_plan_report, discover_python_models, find_project_root, init_db, Config,
    ModelDiscovery,
};
use smelt_core::graph::DependencyGraph;

/// Run the real discovery + Salsa pipeline for `project_dir` and return the
/// built maintenance-plan report for `model_name`, mirroring
/// `commands::explain::explain_maintenance_plan`'s own resolution sequence
/// (find_project_root → Config::load → ModelDiscovery → init_db →
/// Workspace/project → compute_scope + resolve_argument → source_file →
/// smelt_db::maintenance_plan_report → build_maintenance_plan_report).
fn build_report_for(project_dir: &Path, model_name: &str) -> Option<String> {
    let project_dir = find_project_root(project_dir).expect("find project root");
    let config = Config::load(&project_dir).expect("load smelt.yml");
    let sources = smelt_cli::SourcesConfig::load(&project_dir).ok();

    let discovery = ModelDiscovery::new(project_dir.clone(), config.paths.clone());
    let mut models = discovery.discover_models().expect("discover models");

    let python_files = discovery
        .discover_python_files()
        .expect("scan python files");
    if !python_files.is_empty() {
        let python_models = discover_python_models(
            &python_files,
            &models,
            &config,
            &project_dir,
            config.python.as_deref(),
        )
        .expect("discover python models");
        models.extend(python_models);
    }

    let db = init_db(&project_dir, &models);
    let ws = smelt_db::Workspace::try_get(&db).expect("workspace not initialized");
    let project = db
        .project_input(&project_dir)
        .expect("project not initialized");

    let cwd = std::env::current_dir().unwrap_or_else(|_| project_dir.clone());
    let active_scope = compute_scope(&project_dir, &cwd, &config.paths, None);
    let canonical = resolve_argument(&db, ws, project, active_scope.as_ref(), model_name)
        .unwrap_or_else(|e| panic!("resolve_argument({model_name}): {e}"));

    let model = models
        .iter()
        .find(|m| m.canonical_path() == canonical)
        .unwrap_or_else(|| panic!("model '{canonical}' not found among discovered models"));

    let file = db
        .source_file(&model.path)
        .expect("model file not registered");

    let result = smelt_db::maintenance_plan_report(&db, ws, file)?;

    let graph = DependencyGraph::build(models.clone(), sources.as_ref()).expect("build graph");
    let upstream = graph.get_upstream(&canonical);

    let source_infos = smelt_core::discover_source_infos(&project_dir, &config.paths);
    let (own_contract, edges) =
        smelt_cli::explain::build_relation_contract(model, &models, &upstream, &source_infos);

    let maintenance_cfg = model
        .metadata
        .as_deref()
        .and_then(|m| m.maintenance.as_ref());
    let cells_cfg: &[smelt_core::config::MaintenanceCellConfig] =
        maintenance_cfg.map(|m| m.cells.as_slice()).unwrap_or(&[]);
    let defaults_cfg = maintenance_cfg.and_then(|m| m.defaults.as_ref());
    let contract_cfg = model.metadata.as_deref().and_then(|m| m.contract.as_ref());

    // Mirrors `commands::explain::explain_maintenance_plan`'s own
    // `edge_delta_types` assembly (`docs/outcomes/20260809-output-delta-
    // typing/outcome.md` phase 10) so this test helper exercises the real
    // production wiring rather than a stubbed empty slice.
    let edge_delta_types: Vec<(String, smelt_logical::analysis::output_delta::OutputDelta)> = {
        let model_edges = smelt_db::model_edges_for(&db, ws, file);
        edges
            .iter()
            .filter_map(|edge| match edge.provider {
                smelt_cli::explain::RelationContractProvider::Model => {
                    let bare_edge_name = edge.name.strip_prefix("models.").unwrap_or(&edge.name);
                    model_edges
                        .iter()
                        .find(|me| {
                            me.name.strip_prefix("models.").unwrap_or(&me.name) == bare_edge_name
                        })
                        .and_then(|me| me.output_shape.clone())
                        .map(|shape| (edge.name.clone(), shape))
                }
                smelt_cli::explain::RelationContractProvider::Source => {
                    let bare_name = edge.name.strip_prefix("sources.").unwrap_or(&edge.name);
                    smelt_cli::explain::find_source_info(&source_infos, bare_name).map(|info| {
                        let facts =
                            smelt_logical::analysis::output_delta::SourceFacts::from_source_info(
                                bare_name, info,
                            );
                        (
                            edge.name.clone(),
                            smelt_logical::analysis::output_delta::seed_shape_for_source(&facts),
                        )
                    })
                }
            })
            .collect()
    };

    Some(
        build_maintenance_plan_report(
            &canonical,
            &result,
            &own_contract,
            &edges,
            cells_cfg,
            defaults_cfg,
            contract_cfg,
            &source_infos,
            &[],
            smelt_core::config::ProbeCadence::PerRun,
            &edge_delta_types,
            None,
        )
        .expect("build_maintenance_plan_report"),
    )
}

/// `daily_events` in `examples/timeseries` is `refresh: incremental` +
/// `grain: partition` reading a single unclocked-partition source
/// (`raw.events` has no source-level `timeseries:` declaration). The report
/// must name the cell (trigger/corner/technique), print the locality verdict
/// and a scan-clamps section, and the `ledger_catch_up` flag — as data
/// directly readable off `MaintenancePlanResult`, not fabricated.
#[test]
fn explain_prints_cells_clamps_locality() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");

    let report = build_report_for(&project_dir, "daily_events")
        .expect("daily_events has a maintenance plan");

    assert!(
        report.contains("Maintenance plan: daily_events"),
        "{report}"
    );
    assert!(
        report.contains("Cells ("),
        "expected a Cells section: {report}"
    );
    assert!(
        report.contains("trigger"),
        "expected trigger info: {report}"
    );
    assert!(
        report.contains("corner:") && report.contains("technique:"),
        "expected corner/technique per cell: {report}"
    );
    assert!(
        report.contains("locality:"),
        "expected a partition-locality verdict per cell: {report}"
    );
    assert!(
        report.contains("scan clamps"),
        "expected a scan-clamps section per cell: {report}"
    );
    assert!(
        report.contains("ledger_catch_up"),
        "expected the ledger_catch_up flag per cell: {report}"
    );
    assert!(
        report.contains("Inbound edges"),
        "expected an inbound-edges section: {report}"
    );
    // `daily_events` has fully resolved column provenance — the degenerate-
    // collapse callout is a false-positive risk if it were still keyed off
    // "one group spanning 2+ sources" instead of the real `degenerate` signal.
    assert!(
        !report.contains("could not distinguish"),
        "daily_events has no ambiguous provenance; the collapse callout must not fire: {report}"
    );
    assert!(
        report.contains("admissible write patterns: region"),
        "expected the admissible write-pattern registry listing, leading with `region` (the \
         only structural fact this cell's declared facts satisfy first in registry order): \
         {report}"
    );
}

/// A model whose SQL has an unqualified column ambiguous between two joined
/// sources cannot be resolved to a single provenance — the derivation falls
/// back to one column group spanning the whole model
/// (`crates/smelt-logical/tests/maintenance_grouping.rs::degenerate_collapse_is_surfaced`
/// is the pure-derivation-level test for this same shape). The CLI report
/// must call this out in plain language, not silently print an
/// indistinguishable single-group plan.
#[test]
fn degenerate_plan_visibly_reported() {
    let tmp = tempfile::TempDir::new().expect("create tempdir");

    let yml = "name: test_proj\n\
               version: 1\n\
               paths:\n  - models\n\
               targets:\n  dev:\n    type: duckdb\n    schema: main\n\
               default_materialization: view\n";
    std::fs::write(tmp.path().join("smelt.yml"), yml).unwrap();

    std::fs::create_dir_all(tmp.path().join("models/sources")).unwrap();
    std::fs::write(
        tmp.path().join("models/sources/orders.yml"),
        "description: orders\ncolumns:\n- name: order_id\n  type: INTEGER\n- name: order_date\n  type: DATE\n- name: amount\n  type: DECIMAL(10,2)\n- name: user_id\n  type: INTEGER\nmutation_profile:\n  kind: append_only\n",
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("models/sources/customers.yml"),
        "description: customers\ncolumns:\n- name: user_id\n  type: INTEGER\n- name: tier\n  type: VARCHAR\n",
    )
    .unwrap();

    let model_sql = "---\n\
                      materialization: table\n\
                      refresh: incremental\n\
                      grain: key\n\
                      ---\n\
                      SELECT o.order_id, MIN(o.order_date) AS order_date, MIN(amount) AS amount \
                      FROM smelt.sources.orders o \
                      JOIN smelt.sources.customers c ON c.user_id = o.user_id \
                      GROUP BY o.order_id\n";
    std::fs::write(tmp.path().join("models/ambiguous_join.sql"), model_sql).unwrap();

    let report = build_report_for(tmp.path(), "ambiguous_join")
        .expect("ambiguous_join is an incremental model with a maintenance plan");

    assert!(
        report.contains("single column group") || report.contains("could not distinguish"),
        "expected the report to call out the whole-model collapse in plain language: {report}"
    );
}

/// One full CLI-argument-parsing path: spawn the real `smelt` binary with a
/// positional model name and assert the maintenance-plan report (not the
/// whole-project graph JSON) is what prints.
#[test]
fn explain_model_arg_wired_through_real_cli() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");

    let output = Command::new(env!("CARGO_BIN_EXE_smelt"))
        .arg("explain")
        .arg("daily_events")
        .arg("--project-dir")
        .arg(&project_dir)
        .output()
        .expect("spawn smelt explain daily_events");

    assert!(
        output.status.success(),
        "smelt explain daily_events failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Maintenance plan: daily_events"),
        "expected the maintenance-plan report, got: {stdout}"
    );
    assert!(
        !stdout.contains("Logical Graph:"),
        "explain <model> must not fall through to the whole-project graph output: {stdout}"
    );
}

/// `smelt explain` with no model argument keeps printing the whole-project
/// graph — unchanged by this phase.
#[test]
fn explain_without_model_arg_prints_whole_project_graph() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");

    let output = Command::new(env!("CARGO_BIN_EXE_smelt"))
        .arg("explain")
        .arg("--project-dir")
        .arg(&project_dir)
        .output()
        .expect("spawn smelt explain");

    assert!(
        output.status.success(),
        "smelt explain failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Logical Graph:"),
        "expected the whole-project graph output, got: {stdout}"
    );
}

const STATE_COLUMNS_SMELT_YML: &str = "name: state_columns_fixture\n\
    version: 1\n\
    paths:\n  - models\n\
    targets:\n  dev:\n    type: duckdb\n    schema: main\n\
    default_materialization: view\n";

const STATE_COLUMNS_EVENTS_SOURCE: &str = "description: events\n\
    mutation_profile: append_only\n\
    timeseries:\n  event_time_column: event_date\n  partition_column: event_date\n  granularity: day\n\
    columns:\n\
    - name: device_id\n  type: INTEGER\n\
    - name: event_date\n  type: DATE\n\
    - name: amount\n  type: DOUBLE\n";

/// `smelt explain <model>` for a keyed `AVG` model prints an internal-state
/// section naming both hidden state columns and says they are not in the
/// model's public schema (`docs/outcomes/20260809-rung2-state-shapes` row 9).
#[test]
fn explain_renders_internal_state_section() {
    let tmp = tempfile::TempDir::new().expect("create tempdir");
    std::fs::write(tmp.path().join("smelt.yml"), STATE_COLUMNS_SMELT_YML).unwrap();
    std::fs::create_dir_all(tmp.path().join("models/sources")).unwrap();
    std::fs::write(
        tmp.path().join("models/sources/events.yml"),
        STATE_COLUMNS_EVENTS_SOURCE,
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("models/avg_amount.sql"),
        "---\nmaterialization: table\nrefresh: incremental\ngrain: key\n---\n\
         SELECT device_id, AVG(amount) AS avg_amount\n\
         FROM smelt.sources.events\nGROUP BY device_id\n",
    )
    .unwrap();

    let report =
        build_report_for(tmp.path(), "avg_amount").expect("avg_amount has a maintenance plan");

    assert!(
        report.contains("State columns:"),
        "expected an internal-state section: {report}"
    );
    assert!(
        report.contains("avg_amount__sum") && report.contains("avg_amount__count"),
        "expected both hidden state columns named: {report}"
    );
    assert!(
        report.contains("not part of the model's public schema"),
        "expected the state section to say it is not part of the public schema: {report}"
    );
}

/// A keyed `SUM` model has no decomposed state — the report has no state
/// section at all (no empty header).
#[test]
fn explain_omits_state_section_when_no_state() {
    let tmp = tempfile::TempDir::new().expect("create tempdir");
    std::fs::write(tmp.path().join("smelt.yml"), STATE_COLUMNS_SMELT_YML).unwrap();
    std::fs::create_dir_all(tmp.path().join("models/sources")).unwrap();
    std::fs::write(
        tmp.path().join("models/sources/events.yml"),
        STATE_COLUMNS_EVENTS_SOURCE,
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("models/total_amount.sql"),
        "---\nmaterialization: table\nrefresh: incremental\ngrain: key\n---\n\
         SELECT device_id, SUM(amount) AS total_amount\n\
         FROM smelt.sources.events\nGROUP BY device_id\n",
    )
    .unwrap();

    let report =
        build_report_for(tmp.path(), "total_amount").expect("total_amount has a maintenance plan");

    assert!(
        !report.contains("State columns:"),
        "a stateless model must print no state section: {report}"
    );
}

/// `--json` carries the same state-column information as the text section,
/// in a top-level `state_columns` array.
#[test]
fn explain_json_reports_state_columns() {
    let tmp = tempfile::TempDir::new().expect("create tempdir");
    std::fs::write(tmp.path().join("smelt.yml"), STATE_COLUMNS_SMELT_YML).unwrap();
    std::fs::create_dir_all(tmp.path().join("models/sources")).unwrap();
    std::fs::write(
        tmp.path().join("models/sources/events.yml"),
        STATE_COLUMNS_EVENTS_SOURCE,
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("models/avg_amount.sql"),
        "---\nmaterialization: table\nrefresh: incremental\ngrain: key\n---\n\
         SELECT device_id, AVG(amount) AS avg_amount\n\
         FROM smelt.sources.events\nGROUP BY device_id\n",
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_smelt"))
        .arg("explain")
        .arg("avg_amount")
        .arg("--json")
        .arg("--project-dir")
        .arg(tmp.path())
        .output()
        .expect("spawn smelt explain avg_amount --json");

    assert!(
        output.status.success(),
        "smelt explain avg_amount --json failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("expected valid JSON: {e}: {stdout}"));

    let state_columns = json
        .get("state_columns")
        .and_then(|v| v.as_array())
        .unwrap_or_else(|| panic!("expected a top-level state_columns array: {stdout}"));
    assert_eq!(state_columns.len(), 1, "state_columns: {stdout}");
    let entry = &state_columns[0];
    assert_eq!(entry["presented_column"], "avg_amount");
    assert_eq!(
        entry["state_columns"],
        serde_json::json!(["avg_amount__sum", "avg_amount__count"])
    );
    assert_eq!(
        entry["presentation_expr"],
        "avg_amount__sum / avg_amount__count"
    );
}

/// `smelt explain <model>` for a keyed model prints an `Execution postures:`
/// block naming the run shape and all three derived verdicts
/// (`docs/outcomes/20260815-keyed-grain-residue` phase 4).
#[test]
fn explain_prints_execution_postures_for_keyed_model() {
    let tmp = tempfile::TempDir::new().expect("create tempdir");
    std::fs::write(tmp.path().join("smelt.yml"), STATE_COLUMNS_SMELT_YML).unwrap();
    std::fs::create_dir_all(tmp.path().join("models/sources")).unwrap();
    std::fs::write(
        tmp.path().join("models/sources/events.yml"),
        STATE_COLUMNS_EVENTS_SOURCE,
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("models/total_amount.sql"),
        "---\nmaterialization: table\nrefresh: incremental\ngrain: key\n---\n\
         SELECT device_id, SUM(amount) AS total_amount\n\
         FROM smelt.sources.events\nGROUP BY device_id\n",
    )
    .unwrap();

    let report =
        build_report_for(tmp.path(), "total_amount").expect("total_amount has a maintenance plan");

    assert!(
        report.contains("Execution postures:"),
        "expected an execution-postures section: {report}"
    );
    assert!(
        report.contains("run shape: window-forward"),
        "expected the window-forward run shape (clocked source): {report}"
    );
    assert!(
        report.contains("re-run tolerance: no"),
        "SUM is additive, not re-run tolerant: {report}"
    );
    assert!(
        report.contains("order-independence: yes"),
        "SUM's `+` is order-independent: {report}"
    );
    assert!(
        report.contains("reprocessing: refused"),
        "reprocessing refusal is unconditional: {report}"
    );
}

/// `--json` carries the same three verdicts as the text section, in a
/// top-level `execution_postures` object.
#[test]
fn explain_json_carries_execution_postures() {
    let tmp = tempfile::TempDir::new().expect("create tempdir");
    std::fs::write(tmp.path().join("smelt.yml"), STATE_COLUMNS_SMELT_YML).unwrap();
    std::fs::create_dir_all(tmp.path().join("models/sources")).unwrap();
    std::fs::write(
        tmp.path().join("models/sources/events.yml"),
        STATE_COLUMNS_EVENTS_SOURCE,
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("models/total_amount.sql"),
        "---\nmaterialization: table\nrefresh: incremental\ngrain: key\n---\n\
         SELECT device_id, SUM(amount) AS total_amount\n\
         FROM smelt.sources.events\nGROUP BY device_id\n",
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_smelt"))
        .arg("explain")
        .arg("total_amount")
        .arg("--json")
        .arg("--project-dir")
        .arg(tmp.path())
        .output()
        .expect("spawn smelt explain total_amount --json");

    assert!(
        output.status.success(),
        "smelt explain total_amount --json failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("expected valid JSON: {e}: {stdout}"));

    let postures = json
        .get("execution_postures")
        .unwrap_or_else(|| panic!("expected a top-level execution_postures object: {stdout}"));
    assert_eq!(postures["run_shape"], "window-forward");
    assert_eq!(postures["rerun_tolerant"]["holds"], false);
    assert_eq!(postures["order_independent"]["holds"], true);
    assert_eq!(postures["reprocessing_refused"]["holds"], true);
}

/// A `grain: partition` model never classifies through the keyed
/// classifier, so `result.execution_postures` is `None` — the report
/// prints no `Execution postures:` block at all.
#[test]
fn explain_omits_execution_postures_for_non_keyed_model() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");

    let report = build_report_for(&project_dir, "daily_events")
        .expect("daily_events has a maintenance plan");

    assert!(
        !report.contains("Execution postures:"),
        "a grain: partition model must print no execution-postures section: {report}"
    );
}

// =============================================================================
// The contract lattice's `smelt explain` surface (`docs/outcomes/20260809-
// contract-lattice-v1/phases/07-plan.md`): the effective contract per cell —
// default or a relaxed point with its declared parameters — resolved
// through the single-owner `smelt_logical::contract::effective_contract`,
// never a local model-vs-cell ladder.
// =============================================================================

/// `daily_events` declares no `contract:` block — every cell's block prints
/// `contract:  default`.
#[test]
fn explain_prints_default_contract_point_per_cell() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");

    let report = build_report_for(&project_dir, "daily_events")
        .expect("daily_events has a maintenance plan");

    assert!(
        report.contains("contract:  default"),
        "expected a default contract row per cell: {report}"
    );
}

/// `daily_event_counts_frozen` in `examples/timeseries` declares
/// `contract.frozen_horizon: '365 days'` — the report renders it on the
/// model's cell.
#[test]
fn explain_prints_frozen_horizon_contract_point() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");

    let report = build_report_for(&project_dir, "daily_event_counts_frozen")
        .expect("daily_event_counts_frozen has a maintenance plan");

    assert!(
        report.contains("contract:  frozen_horizon 365 days"),
        "expected the declared frozen_horizon on the cell's contract row: {report}"
    );
}

/// `--json` carries the same effective contract per cell in a
/// `contract_point` object; a default cell omits the relaxation keys rather
/// than rendering them `null`.
#[test]
fn explain_json_carries_contract_point_per_cell() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");

    let output = Command::new(env!("CARGO_BIN_EXE_smelt"))
        .arg("explain")
        .arg("daily_event_counts_frozen")
        .arg("--json")
        .arg("--project-dir")
        .arg(&project_dir)
        .output()
        .expect("spawn smelt explain daily_event_counts_frozen --json");

    assert!(
        output.status.success(),
        "smelt explain daily_event_counts_frozen --json failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("expected valid JSON: {e}: {stdout}"));

    let cells = json
        .get("cells")
        .and_then(|v| v.as_array())
        .unwrap_or_else(|| panic!("expected a top-level cells array: {stdout}"));
    assert!(!cells.is_empty(), "expected at least one cell: {stdout}");
    for cell in cells {
        let contract_point = cell
            .get("contract_point")
            .unwrap_or_else(|| panic!("expected contract_point on every cell: {stdout}"));
        assert_eq!(
            contract_point
                .get("frozen_horizon")
                .and_then(|v| v.as_str()),
            Some("365 days"),
            "expected the declared frozen_horizon on contract_point: {stdout}"
        );
        // No deferral is declared — those keys are omitted, never null.
        assert!(
            contract_point.get("deferral").is_none(),
            "an undeclared relaxation must be omitted, not rendered null: {stdout}"
        );
        assert!(contract_point.get("deferral_origin").is_none());
    }

    let default_output = Command::new(env!("CARGO_BIN_EXE_smelt"))
        .arg("explain")
        .arg("daily_events")
        .arg("--json")
        .arg("--project-dir")
        .arg(&project_dir)
        .output()
        .expect("spawn smelt explain daily_events --json");
    assert!(default_output.status.success());
    let default_stdout = String::from_utf8_lossy(&default_output.stdout);
    let default_json: serde_json::Value = serde_json::from_str(&default_stdout)
        .unwrap_or_else(|e| panic!("expected valid JSON: {e}: {default_stdout}"));
    let default_cells = default_json
        .get("cells")
        .and_then(|v| v.as_array())
        .unwrap_or_else(|| panic!("expected a top-level cells array: {default_stdout}"));
    for cell in default_cells {
        let contract_point = cell
            .get("contract_point")
            .unwrap_or_else(|| panic!("expected contract_point on every cell: {default_stdout}"));
        assert_eq!(
            contract_point.as_object().map(|o| o.len()),
            Some(0),
            "a default cell's contract_point must be an empty object, not null-filled keys: \
             {default_stdout}"
        );
    }
}

// =============================================================================
// The repair family's surface (`docs/outcomes/20260809-repair-family/phases/
// 11-plan.md`): a `Technique::PerGroupRecompute` cell's own report stanza —
// its affected-key slice, bounded per-group read bound, affected-key
// discovery mechanism, and (for a `write: diff_patch` pin) the resolved
// write mechanism and delete-leg verdict. `smelt_maintenance_testkit::
// recipe::RepairRecipe` stages the same non-invertible-fold-over-a-mutable-
// clocked-source shape `repair_lowering.rs` hand-builds, generalized into
// typed recipe data.
// =============================================================================

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
    };
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

/// Stage a project with a clocked keyed upstream (`dag_kchain_a`, `KeyedAgg`
/// over the append-only `events` source, grouped by `id` — no clock of its
/// own) feeding a keyed-fold downstream (`dag_kchain_b`) — the generated
/// [`smelt_maintenance_testkit::dag::keyed_chain_dag`] fixture phase 6/7/8
/// already prove end-to-end for execution; here it is only staged (no run)
/// to exercise `smelt explain`'s own report.
fn stage_keyed_chain_project(tmp: &tempfile::TempDir) -> std::path::PathBuf {
    use smelt_maintenance_testkit::dag::{keyed_chain_dag, stage_dag};

    let project_dir = tmp.path().join("project");
    let db_path = tmp.path().join("db.duckdb");
    let dag = keyed_chain_dag();
    stage_dag(&dag, &project_dir, &db_path).expect("stage keyed chain dag");
    project_dir
}

/// `dag_kchain_a` (clockless, `KeyedUpsert`-shaped via its own `GROUP BY id`
/// over an append-only source) is `dag_kchain_b`'s only inbound edge — its
/// block must print `delta type: keyed upsert`.
#[test]
fn explain_renders_keyed_upsert_edge_delta_type() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = stage_keyed_chain_project(&tmp);

    let report = build_report_for(&project_dir, "dag_kchain_b")
        .expect("dag_kchain_b has a maintenance plan");

    assert!(
        report.contains("dag_kchain_a (model)"),
        "expected dag_kchain_a as an inbound model edge: {report}"
    );
    assert!(
        report.contains("delta type: keyed upsert"),
        "expected the clockless keyed upstream's edge to be typed keyed upsert: {report}"
    );
}

/// `user_daily_spend` in `examples/timeseries` reads the clocked,
/// `append_only` `sources.raw.transactions` — the common case's edge must
/// still print `delta type: append-only within window` (no regression from
/// the new row).
#[test]
fn explain_renders_append_only_window_edge_delta_type() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");

    let report = build_report_for(&project_dir, "user_daily_spend")
        .expect("user_daily_spend has a maintenance plan");

    assert!(
        report.contains("sources.raw.transactions (source)"),
        "expected sources.raw.transactions as an inbound source edge: {report}"
    );
    assert!(
        report.contains("delta type: append-only within window"),
        "expected the clocked append-only source edge to be typed append-only within window: \
         {report}"
    );
}

/// Stage a project with: a clocked append-only `events` source; an
/// `undeclared` source with no `mutation_profile`; `windowed_upstream`
/// (refresh: incremental, a window-function output column — an operator the
/// walk cannot classify as addressable); `general_consumer` (reads
/// `windowed_upstream`); `source_consumer` (reads `sources.undeclared`);
/// `view_upstream` (plain view, no `refresh: incremental` at all); and
/// `view_consumer` (reads `view_upstream`) — one project reused across the
/// `general`/absent-verdict edge-typing tests below.
fn stage_delta_type_project(tmp: &tempfile::TempDir) -> std::path::PathBuf {
    let project_dir = tmp.path().join("project");
    std::fs::create_dir_all(project_dir.join("models/sources")).expect("create dirs");

    std::fs::write(
        project_dir.join("smelt.yml"),
        "name: delta_type_fixture\n\
         version: 1\n\
         paths:\n  - models\n\
         targets:\n  dev:\n    type: duckdb\n    schema: main\n\
         default_materialization: view\n",
    )
    .expect("write smelt.yml");

    std::fs::write(
        project_dir.join("models/sources/events.yml"),
        "description: clocked append-only test source\n\
         mutation_profile: append_only\n\
         timeseries:\n  event_time_column: d\n  partition_column: d\n  granularity: day\n\
         columns:\n\
         - name: d\n  type: DATE\n\
         - name: id\n  type: INTEGER\n\
         - name: val\n  type: INTEGER\n",
    )
    .expect("write events.yml");

    std::fs::write(
        project_dir.join("models/sources/undeclared.yml"),
        "description: test source declaring no mutation_profile\n\
         columns:\n\
         - name: d\n  type: DATE\n\
         - name: id\n  type: INTEGER\n\
         - name: val\n  type: INTEGER\n",
    )
    .expect("write undeclared.yml");

    let ts_frontmatter = "---\ntimeseries:\n  event_time_column: d\n  partition_column: d\n  \
                           granularity: day\nrefresh: incremental\ngrain: partition\n---\n";

    std::fs::write(
        project_dir.join("models/windowed_upstream.sql"),
        format!(
            "{ts_frontmatter}SELECT d, id, val, ROW_NUMBER() OVER (PARTITION BY id ORDER BY d) \
             AS rn\nFROM smelt.sources.events\n"
        ),
    )
    .expect("write windowed_upstream.sql");

    std::fs::write(
        project_dir.join("models/general_consumer.sql"),
        format!("{ts_frontmatter}SELECT d, id, val, rn\nFROM smelt.windowed_upstream\n"),
    )
    .expect("write general_consumer.sql");

    std::fs::write(
        project_dir.join("models/source_consumer.sql"),
        format!("{ts_frontmatter}SELECT d, id, val\nFROM smelt.sources.undeclared\n"),
    )
    .expect("write source_consumer.sql");

    std::fs::write(
        project_dir.join("models/view_upstream.sql"),
        "SELECT d, id, val FROM smelt.sources.events\n",
    )
    .expect("write view_upstream.sql");

    std::fs::write(
        project_dir.join("models/view_consumer.sql"),
        format!("{ts_frontmatter}SELECT d, id, val\nFROM smelt.view_upstream\n"),
    )
    .expect("write view_consumer.sql");

    project_dir
}

/// `windowed_upstream`'s `rn` column is a window-function output — the walk
/// cannot classify it as addressable, so `general_consumer`'s inbound edge to
/// it must print `delta type: general` naming the window-function construct.
#[test]
fn explain_names_construct_that_degraded_edge_delta_type() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = stage_delta_type_project(&tmp);

    let report = build_report_for(&project_dir, "general_consumer")
        .expect("general_consumer has a maintenance plan");

    assert!(
        report.contains("windowed_upstream (model)"),
        "expected windowed_upstream as an inbound model edge: {report}"
    );
    assert!(
        report.contains("delta type: general (degraded by:") && report.contains("window-function"),
        "expected a general verdict naming the window-function construct: {report}"
    );
}

/// `sources.undeclared` declares no `mutation_profile` — the fail-closed
/// seed must be visible (`general`), not silently skipped, and the reason
/// must name the missing declaration.
#[test]
fn explain_renders_source_edge_delta_type() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = stage_delta_type_project(&tmp);

    let report = build_report_for(&project_dir, "source_consumer")
        .expect("source_consumer has a maintenance plan");

    assert!(
        report.contains("sources.undeclared (source)"),
        "expected sources.undeclared as an inbound source edge: {report}"
    );
    assert!(
        report.contains("delta type: general (degraded by:")
            && report.contains("declares no mutation_profile"),
        "expected a general verdict naming the missing mutation_profile declaration: {report}"
    );
}

/// `view_upstream` is a plain view (no `refresh: incremental`) — it
/// contributes no [`smelt_db::model_edges_for`] entry, so `view_consumer`'s
/// edge to it must print no `delta type:` row at all rather than a
/// fabricated one.
#[test]
fn explain_edge_without_derived_shape_prints_no_delta_row() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = stage_delta_type_project(&tmp);

    let report = build_report_for(&project_dir, "view_consumer")
        .expect("view_consumer has a maintenance plan");

    assert!(
        report.contains("view_upstream (model)"),
        "expected view_upstream as an inbound model edge: {report}"
    );
    let edge_block_start = report
        .find("view_upstream (model)")
        .expect("view_upstream edge block present");
    let edge_block = &report[edge_block_start..];
    let edge_block_end = edge_block
        .find("\n\n")
        .map(|i| edge_block_start + i)
        .unwrap_or(report.len());
    assert!(
        !report[edge_block_start..edge_block_end].contains("delta type:"),
        "a non-incremental upstream's edge must print no delta type row: {report}"
    );
}

/// `dag_kchain_b`'s `PerGroupRecompute` cell over the clockless keyed
/// upstream `dag_kchain_a` is key-addressed (`cell.key_scope`) — its repair
/// stanza's affected-key discovery line must name the group-grain
/// fingerprint-sidecar diff over the upstream's own output table, not the
/// declared-source discovery mechanism (`dag_kchain_a` is a model, not a
/// declared source).
#[test]
fn explain_key_addressed_cell_prints_upstream_sidecar_discovery() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = stage_keyed_chain_project(&tmp);

    let report = build_report_for(&project_dir, "dag_kchain_b")
        .expect("dag_kchain_b has a maintenance plan");

    assert!(
        report.contains("technique: PerGroupRecompute"),
        "expected a key-addressed PerGroupRecompute cell: {report}"
    );
    assert!(
        report.contains(
            "affected-key discovery: group-grain fingerprint-sidecar diff over the upstream's \
             own output table"
        ),
        "expected the upstream-sidecar discovery mechanism, not a declared-source posture: \
         {report}"
    );
}

/// Phase 24b (`docs/outcomes/20260815-definition-delta-migrate/phases/
/// 24b-plan.md`): `silver.device_user_edges` regroups
/// `silver.events_deduped`'s rows onto `device_id, user_id` — real columns
/// of the upstream relation, not `events_deduped`'s own `KeyedUpsert` key
/// (`event_id`). The grain-over-upstream discovery route admits this: the
/// cell must resolve with no `RepairKeysNotDiscoverable` refusal, and the
/// report must name the grain-over-upstream route.
#[test]
fn device_user_edges_admits_a_key_addressed_cell() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/web_analytics")
        .canonicalize()
        .expect("examples/web_analytics exists");

    let report = build_report_for(&project_dir, "silver.device_user_edges")
        .expect("silver.device_user_edges has a maintenance plan");

    assert!(
        !report.contains("RepairKeysNotDiscoverable"),
        "device_user_edges must no longer refuse key-addressed admission: {report}"
    );
    assert!(
        report.contains("technique: PerGroupRecompute"),
        "expected a key-addressed PerGroupRecompute cell: {report}"
    );
    assert!(
        report.contains(
            "affected-key discovery: group-grain fingerprint-sidecar diff over the upstream's \
             own output table (keyed at the downstream's own grain, projected over the upstream \
             relation)"
        ),
        "expected the grain-over-upstream discovery route to be named: {report}"
    );
}

/// A `grain: partition` model joining a clocked, append-only driving source
/// with a second, unclocked `mutable_snapshot` source — the second source's
/// scan cannot be partition-bounded, so the plan refuses admission for it
/// (`MaintenanceScanUnbounded`) rather than silently shipping a full-table
/// write (`incremental_models.md` §"Partition-local maintenance (the K8
/// guardrail)").
fn stage_scan_unbounded_project(tmp: &tempfile::TempDir) -> std::path::PathBuf {
    let project_dir = tmp.path().join("project");
    std::fs::create_dir_all(project_dir.join("models/sources")).expect("create dirs");

    std::fs::write(
        project_dir.join("smelt.yml"),
        "name: scan_unbounded_fixture\n\
         version: 1\n\
         paths:\n  - models\n\
         targets:\n  dev:\n    type: duckdb\n    schema: main\n\
         default_materialization: view\n",
    )
    .expect("write smelt.yml");

    std::fs::write(
        project_dir.join("models/sources/clocked.yml"),
        "description: clocked append-only source\n\
         mutation_profile: append_only\n\
         timeseries:\n  event_time_column: d\n  partition_column: d\n  granularity: day\n\
         columns:\n\
         - name: d\n  type: DATE\n\
         - name: id\n  type: INTEGER\n",
    )
    .expect("write clocked.yml");

    std::fs::write(
        project_dir.join("models/sources/unclocked.yml"),
        "description: unclocked mutable source, no clock to bound its scan\n\
         mutation_profile: mutable_snapshot\n\
         columns:\n\
         - name: id\n  type: INTEGER\n\
         - name: val\n  type: INTEGER\n",
    )
    .expect("write unclocked.yml");

    std::fs::write(
        project_dir.join("models/joined.sql"),
        "---\n\
         timeseries:\n  event_time_column: d\n  partition_column: d\n  granularity: day\n\
         refresh: incremental\n\
         grain: partition\n\
         ---\n\
         SELECT c.d, c.id, u.val\n\
         FROM smelt.sources.clocked c\n\
         JOIN smelt.sources.unclocked u ON c.id = u.id\n",
    )
    .expect("write joined.sql");

    project_dir
}

/// `smelt explain <model> --json`'s `refusals` array carries the same
/// admission refusal the text report's "Refusals" section prints — read
/// verbatim from the property profile
/// (`docs/specs/property_diff.md` §"The property profile", test 6 of
/// `docs/outcomes/20260905-property-diff/phases/02-plan.md`).
#[test]
fn explain_json_carries_refusals() {
    let tmp = tempfile::tempdir().expect("create tempdir");
    let project_dir = stage_scan_unbounded_project(&tmp);

    let report =
        build_report_for(&project_dir, "joined").expect("joined has a maintenance plan");
    assert!(
        report.contains("MaintenanceScanUnbounded") || report.contains("ScanUnbounded"),
        "expected the text report to print the ScanUnbounded refusal: {report}"
    );

    let output = Command::new(env!("CARGO_BIN_EXE_smelt"))
        .arg("explain")
        .arg("joined")
        .arg("--json")
        .arg("--project-dir")
        .arg(&project_dir)
        .output()
        .expect("spawn smelt explain joined --json");
    assert!(
        output.status.success(),
        "smelt explain joined --json failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&output.stdout)).expect("parse JSON");

    let refusals = json["refusals"]
        .as_array()
        .expect("refusals must be an array");
    assert!(
        !refusals.is_empty(),
        "expected at least one refusal in --json output: {json}"
    );
    assert!(
        refusals
            .iter()
            .all(|r| r["code"].as_str() == Some("MaintenanceScanUnbounded")),
        "expected every refusal here to be MaintenanceScanUnbounded: {refusals:?}"
    );
    assert!(
        refusals.iter().any(|r| r["text"]
            .as_str()
            .is_some_and(|t| t.contains("ScanUnbounded") && t.contains("unclocked"))),
        "expected the refusal text to name the unclocked source: {refusals:?}"
    );
}
