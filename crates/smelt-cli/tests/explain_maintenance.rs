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

    Some(build_maintenance_plan_report(
        &canonical, &result, &upstream,
    ))
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
                      SELECT o.order_id, o.order_date, amount \
                      FROM smelt.sources.orders o \
                      JOIN smelt.sources.customers c ON c.user_id = o.user_id\n";
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
