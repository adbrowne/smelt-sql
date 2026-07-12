//! `smelt explain <model>` over a real fixture with **model-to-model** edges
//! (`maintenance_plan.md` §"Upstream model edges"): a maintained model's ref
//! to another maintained model derives a creation-trigger cell clocked by the
//! upstream's own `timeseries:` declaration.

use std::path::Path;

use smelt_cli::argument_resolution::{compute_scope, resolve_argument};
use smelt_cli::{
    build_maintenance_plan_report, discover_python_models, find_project_root, init_db, Config,
    ModelDiscovery,
};
use smelt_core::graph::DependencyGraph;

/// Run the real discovery + Salsa pipeline for `project_dir` and return the
/// built maintenance-plan report for `model_name` (mirrors
/// `explain_maintenance.rs::build_report_for` /
/// `commands::explain::explain_maintenance_plan`'s resolution sequence).
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

/// `gold.eventstream_with_identity` in `examples/web_analytics` joins its
/// clocked silver upstream `silver.events_parsed`. The maintenance-plan
/// report must show a creation cell for that model edge.
#[test]
fn eventstream_shows_creation_cell_for_silver_upstream() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/web_analytics")
        .canonicalize()
        .expect("examples/web_analytics exists");

    let report = build_report_for(&project_dir, "gold.eventstream_with_identity")
        .expect("eventstream_with_identity has a maintenance plan");

    assert!(
        report.contains("NewData { source: \"silver.events_parsed\" }"),
        "expected a creation cell for the model upstream silver.events_parsed: {report}"
    );
}

/// `silver.events_enriched` (`docs/plans/20260710-web-analytics-maintenance-demo.md`
/// Phase 7) refs **two** maintained-model upstreams in the same body:
/// `silver.events_parsed` (its own `event_date` clock, read 1:1) and
/// `silver.sessions` (clocked by `session_start_date`, joined across the
/// session boundary via the 1-day session-cap Form B filter). The
/// maintenance-plan report must show a creation cell — each with its own
/// derived scan clamp — for BOTH upstreams, demonstrating that the
/// model-upstream edge derivation (`maintenance_plan.md` §"Upstream model
/// edges") composes across more than one model-to-model ref.
#[test]
fn events_enriched_shows_creation_cells_for_both_model_upstreams() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/web_analytics")
        .canonicalize()
        .expect("examples/web_analytics exists");

    let report = build_report_for(&project_dir, "silver.events_enriched")
        .expect("events_enriched has a maintenance plan");

    assert!(
        report.contains("NewData { source: \"silver.events_parsed\" }"),
        "expected a creation cell for the model upstream silver.events_parsed: {report}"
    );
    assert!(
        report.contains("NewData { source: \"silver.sessions\" }"),
        "expected a creation cell for the model upstream silver.sessions: {report}"
    );
}

/// `silver.sessions`'s outermost FROM is a `TableExpr`-returning function
/// call (`smelt.functions.sessionize(...)`, nested inside a CTE), and its
/// `partition_column` (`session_start_date`) skews the driving `event_date`
/// column forward by one day (a Form B relation) — the derived output
/// window for a requested `[2026-04-10, 2026-04-11)` run is the skew-inverted
/// `[2026-04-09, 2026-04-11)` (`docs/specs/model_transforms.md` §Semantics
/// "The output window is derived, never assumed").
///
/// `smelt explain silver.sessions --show-sql --json --period …` must emit a
/// non-empty DELETE+INSERT statement group for this model — not the
/// "failed to inject the output clamp: No FROM clause found" refusal a
/// compile-then-clamp ordering bug produced against a `TableExpr` FROM — with
/// the DELETE range and output clamp at the skew-inverted window and the
/// source-scan pushdown filter widened a further two days backward and one
/// day forward beyond it (`[2026-04-07, 2026-04-12)`, `sessionize`'s own
/// `max_lookback` reach applied on top of the already-widened output
/// window, per the two-layer widened-scan design). No backend is opened —
/// `--show-sql` never connects to one.
#[test]
fn sessions_show_sql_emits_statements() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/web_analytics")
        .canonicalize()
        .expect("examples/web_analytics exists");

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_smelt"))
        .arg("explain")
        .arg("silver.sessions")
        .arg("--show-sql")
        .arg("--json")
        .arg("--period")
        .arg("2026-04-10..2026-04-11")
        .arg("--project-dir")
        .arg(&project_dir)
        .output()
        .expect("spawn smelt explain silver.sessions --show-sql --json");

    assert!(
        output.status.success(),
        "smelt explain --show-sql failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("invalid --json output: {e}\n{stdout}"));

    let cells = json["cells"]
        .as_array()
        .unwrap_or_else(|| panic!("expected a `cells` array: {stdout}"));
    assert!(
        !cells.is_empty(),
        "expected at least one plan cell for silver.sessions: {stdout}"
    );

    let mut saw_delete_insert_group = false;
    for cell in cells {
        assert!(
            cell["no_statements_reason"].is_null(),
            "cell {:?} refused statements: {:?}\nfull output: {stdout}",
            cell["trigger"],
            cell["no_statements_reason"]
        );
        let statements = cell["statements"].as_array().unwrap_or_else(|| {
            panic!(
                "expected a `statements` array for cell {:?}: {stdout}",
                cell["trigger"]
            )
        });
        assert!(
            !statements.is_empty(),
            "expected non-empty statements for cell {:?}: {stdout}",
            cell["trigger"]
        );

        let joined: String = statements
            .iter()
            .map(|s| s["sql"].as_str().unwrap_or_default())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(
            joined.contains(
                "DELETE FROM main.silver_sessions WHERE session_start_date >= '2026-04-09' \
                 AND session_start_date < '2026-04-11'"
            ),
            "expected the skew-inverted DELETE range [2026-04-09, 2026-04-11) for cell {:?}: {joined}",
            cell["trigger"]
        );
        assert!(
            joined.contains(
                "_smelt_output_clamp WHERE session_start_date >= '2026-04-09' \
                 AND session_start_date < '2026-04-11'"
            ),
            "expected the skew-inverted output clamp [2026-04-09, 2026-04-11) for cell {:?}: {joined}",
            cell["trigger"]
        );
        assert!(
            joined.contains(
                "main.silver_events_parsed WHERE event_date >= '2026-04-07' \
                 AND event_date < '2026-04-12'"
            ),
            "expected the widened source scan [2026-04-07, 2026-04-12) for cell {:?}: {joined}",
            cell["trigger"]
        );
        saw_delete_insert_group = true;
    }
    assert!(
        saw_delete_insert_group,
        "expected at least one DeleteInsert cell to check: {stdout}"
    );
}
