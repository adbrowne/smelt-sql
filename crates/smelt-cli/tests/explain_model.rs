//! `smelt explain <model>` over a real fixture with **model-to-model** edges
//! (`incremental_models.md` §"Upstream model edges"): a maintained model's ref
//! to another maintained model derives a creation-trigger cell clocked by the
//! upstream's own `timeseries:` declaration.

use std::path::Path;

use smelt_cli::argument_resolution::{compute_scope, resolve_argument};
use smelt_cli::{
    build_maintenance_plan_report, discover_python_models, find_project_root, init_db, Config,
    ModelDiscovery,
};
use smelt_core::graph::DependencyGraph;

/// Build a minimal [`smelt_logical::analysis::profile::PropertyProfile`]
/// over a hand-constructed [`smelt_db::queries::maintenance::
/// MaintenancePlanResult`] for a test that fabricates its own plan rather
/// than deriving one from real SQL — `PropertySet::derive` needs *some*
/// SQL, so this uses a one-column stand-in; only `cell_verdicts` (built
/// from `result`'s own cells) and `refusals` matter to these tests'
/// assertions.
fn synthetic_profile(
    result: &smelt_db::queries::maintenance::MaintenancePlanResult,
    model_name: &str,
) -> smelt_logical::analysis::profile::PropertyProfile {
    let properties = smelt_logical::analysis::profile::PropertySet::derive(
        model_name,
        "SELECT 1 AS c",
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
    smelt_logical::analysis::profile::PropertyProfile::assemble(
        properties,
        &result.plan.cells,
        &contract_points,
        &result.plan.refusals,
        &[],
    )
}

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
    // typing/outcome.md` phase 10).
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

    let bound_ctx = smelt_cli::explain::build_bound_context(&canonical, &graph, &config);
    let profile = smelt_runtime::diagnostics::build_model_profile(
        model,
        &bound_ctx,
        &result.plan.cells,
        &result.column_groups,
        &result.plan.refusals,
        &[],
        contract_cfg,
    )
    .expect("build_model_profile");

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
            None,
            &profile,
        )
        .expect("build_maintenance_plan_report"),
    )
}

/// `gold.eventstream_with_identity` in `examples/web_analytics` joins its
/// clocked silver upstream `silver.events_deduped`. The maintenance-plan
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
        report.contains("NewData { source: \"silver.events_deduped\" }"),
        "expected a creation cell for the model upstream silver.events_deduped: {report}"
    );
}

/// `silver.events_enriched` (`docs/plans/20260710-web-analytics-maintenance-demo.md`
/// Phase 7) refs **two** maintained-model upstreams in the same body:
/// `silver.events_deduped` (the composed keyed+timeseries dedupe stage,
/// read 1:1) and `silver.sessions` (clocked by `session_start_date`, joined
/// across the session boundary via the 1-day session-cap Form B filter).
/// The maintenance-plan report must show a creation cell — each with its
/// own derived scan clamp — for BOTH upstreams, demonstrating that the
/// model-upstream edge derivation (`incremental_models.md` §"Upstream model
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
        report.contains("NewData { source: \"silver.events_deduped\" }"),
        "expected a creation cell for the model upstream silver.events_deduped: {report}"
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
/// source-scan pushdown filter (against `silver.events_deduped`, the
/// composed keyed+timeseries dedupe stage — `docs/specs/
/// incremental_shapes.md` §"Key temporal locality") widened a further two
/// days backward beyond it (`[2026-04-07, 2026-04-11)`, `sessionize`'s own
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
                "main.silver_events_deduped WHERE first_seen_date >= '2026-04-07' \
                 AND first_seen_date < '2026-04-11'"
            ),
            "expected the widened source scan [2026-04-07, 2026-04-11) for cell {:?}: {joined}",
            cell["trigger"]
        );
        saw_delete_insert_group = true;
    }
    assert!(
        saw_delete_insert_group,
        "expected at least one DeleteInsert cell to check: {stdout}"
    );
}

/// Phase A0 TDD (`docs/plans/20260715-composed-axes-conditional-maintenance.md`):
/// `smelt explain` on `examples/timeseries_broken_key_per_partition/models/trajectory.sql`
/// (a `timeseries:` clock plus a `unique_key:` identity whose
/// `partition_column` is a member — derived `key_per_partition`) prints the
/// `UnsupportedGrain` refusal — naming the grain and the tracking plan — and
/// no cell table, never a keyed cell derived with an empty `unique_key`.
#[test]
fn key_per_partition_shows_unsupported_grain_refusal_not_keyed_cells() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries_broken_key_per_partition")
        .canonicalize()
        .expect("examples/timeseries_broken_key_per_partition exists");

    let report =
        build_report_for(&project_dir, "trajectory").expect("trajectory has a maintenance plan");

    assert!(
        report.contains("Cells: (none)"),
        "expected no cells for the unsupported key_per_partition grain: {report}"
    );
    assert!(
        report.contains("UnsupportedGrain"),
        "expected the UnsupportedGrain refusal: {report}"
    );
    assert!(
        report.contains("key_per_partition"),
        "expected the refusal to name the grain: {report}"
    );
    assert!(
        report.contains("20260715-composed-axes-conditional-maintenance.md"),
        "expected the refusal to name the tracking plan: {report}"
    );
}

/// Phase A5 (`docs/plans/20260715-composed-axes-conditional-maintenance.md`):
/// `examples/timeseries/models/user_daily_spend.sql` is a locality-admitted
/// composed model (`grain: key` + `timeseries:`, route 1 key-embedded).
/// `examples/timeseries/models/user_spend_rollup.sql` is an ordinary
/// `grain: partition` downstream that reads it with a genuine bounded
/// lookback. `incremental_shapes.md` §"Key temporal locality (the
/// time-partitioned output)" — "The output as a clocked source": the
/// composed output must be visible to the rest of the DAG exactly like a
/// declared source, so the downstream's compiled SQL must carry ordinary
/// source-filter pushdown against it (the 3-day lookback widening the read
/// window), not a full unclamped scan (the "clock-sink" the spec warns a
/// keyed stage can otherwise become). `--show-sql` never connects to a
/// backend.
#[test]
fn downstream_partition_grain_model_gets_pushdown_against_a_composed_upstream() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_smelt"))
        .arg("explain")
        .arg("user_spend_rollup")
        .arg("--show-sql")
        .arg("--json")
        .arg("--period")
        .arg("2024-01-05..2024-01-06")
        .arg("--project-dir")
        .arg(&project_dir)
        .output()
        .expect("spawn smelt explain user_spend_rollup --show-sql --json");

    assert!(
        output.status.success(),
        "smelt explain user_spend_rollup --show-sql failed: stderr={}",
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
        "expected at least one plan cell for user_spend_rollup: {stdout}"
    );

    // The upstream model edge itself must surface as a creation-trigger
    // cell — the model-to-model edge derivation
    // (`incremental_models.md` §"Upstream model edges") applies to a
    // composed upstream exactly as it would to any other maintained model.
    assert!(
        cells.iter().any(|c| c["trigger"]
            .as_str()
            .is_some_and(|t| t == "NewData { source: \"user_daily_spend\" }")),
        "expected a creation cell for the composed upstream user_daily_spend: {stdout}"
    );

    let mut saw_pushdown = false;
    for cell in cells {
        let statements = cell["statements"].as_array().unwrap_or_else(|| {
            panic!(
                "expected a `statements` array for cell {:?}: {stdout}",
                cell["trigger"]
            )
        });
        let joined: String = statements
            .iter()
            .map(|s| s["sql"].as_str().unwrap_or_default())
            .collect::<Vec<_>>()
            .join("\n");
        // The requested output window is [2024-01-05, 2024-01-06); the
        // composed upstream's own read is widened 3 days backward by the
        // downstream's literal `INTERVAL '3 days'` lookback (Form B) —
        // [2024-01-02, 2024-01-06). This is ordinary source-filter
        // pushdown, identical in shape to pushdown against a declared
        // `timeseries:` source — the clock propagated through the composed
        // stage instead of stopping there.
        if joined.contains(
            "FROM main.user_daily_spend WHERE spend_date >= '2024-01-02' \
             AND spend_date < '2024-01-06'",
        ) {
            saw_pushdown = true;
        }
    }
    assert!(
        saw_pushdown,
        "expected the widened pushdown scan [2024-01-02, 2024-01-06) against the composed \
         upstream user_daily_spend in at least one cell's statements: {stdout}"
    );
}

/// Phase A5: `smelt explain` prints the locality verdict (route, slice
/// form) and the derived settle bound for a composed model
/// (`docs/specs/incremental_shapes.md` §"Key temporal locality (the
/// time-partitioned output)" — "The output's **settle bound**").
/// `examples/timeseries/models/user_daily_spend.sql` admits route 1
/// (key-embedded: `spend_date` is itself a `unique_key` column).
#[test]
fn explain_prints_locality_route_and_settle_bound_for_a_composed_model() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");

    let report =
        build_report_for(&project_dir, "user_daily_spend").expect("model has a maintenance plan");

    assert!(
        report.contains("Key temporal locality:"),
        "expected a locality section in the report: {report}"
    );
    assert!(
        report.contains("route 1"),
        "expected route 1 (key-embedded) named: {report}"
    );
    assert!(
        report.contains("settle bound:"),
        "expected a settle bound line: {report}"
    );
    // Route 1's settle bound is `After { margin: .. }`, never the honest
    // `Never` route 2 alone gets — assert it does NOT print `Never` here,
    // confirming the two routes render distinctly rather than one sentinel
    // shape for both.
    assert!(
        !report.contains("settle bound: Never"),
        "route 1 must not print route 2's honest `Never` sentinel: {report}"
    );
}

/// Phase A5, continued: `examples/timeseries/models/user_spend_running_total.sql`
/// is a downstream **keyed** model whose driving source is
/// `user_daily_spend`'s own composed output, not a declared source — its
/// own locality must also establish (route 1, since `spend_date` is a
/// `unique_key` column of the downstream too), proving the clock
/// propagates window-forward through the composed stage instead of
/// stopping at it.
#[test]
fn downstream_keyed_model_selects_composed_upstream_as_driving_source_via_explain() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");

    let report = build_report_for(&project_dir, "user_spend_running_total")
        .expect("model has a maintenance plan");

    assert!(
        !report.contains("Refusals (1)") && !report.contains("LocalityNotEstablished"),
        "the composed upstream's own output must resolve as this keyed model's driving \
         source (route 1) — no locality refusal expected: {report}"
    );
    assert!(
        report.contains("Key temporal locality:"),
        "expected a locality section: {report}"
    );
    assert!(
        report.contains("Inbound edges: user_daily_spend"),
        "expected the composed upstream as an inbound edge: {report}"
    );
}

// ---------------------------------------------------------------------------
// explain_prints_relation_contract (Phase S2 of
// `docs/plans/20260715-composed-axes-conditional-maintenance.md`):
// `smelt explain <model>` prints the Relation Contract
// (`docs/specs/models.md` §"The Relation Contract") — the model's own
// clock/identity/derived-grain rows, plus one contract block per inbound
// edge, source and model providers rendered through the same rows.
// ---------------------------------------------------------------------------

/// `daily_events_status` (`examples/timeseries`) directly refs **two**
/// sources with different shapes: `raw.events` (declares `unique_key:
/// [event_id]`, no clock — keyed-dimension) and `raw.user_status` (declares
/// both a clock and `unique_key: [user_id]`, `changed_at` NOT in the key —
/// keyed, time-partitioned). The model's own facts (`timeseries:` only, no
/// top-level `unique_key:`) derive `grain: partition`. All three renders —
/// the model's own contract and both source edges' contracts — must use the
/// identical field names (`clock:`, `identity:`, `derived grain:`).
#[test]
fn explain_prints_relation_contract_for_source_edges() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");

    let report = build_report_for(&project_dir, "daily_events_status")
        .expect("model has a maintenance plan");

    assert!(
        report.contains("Relation contract:"),
        "expected a Relation Contract section: {report}"
    );
    // The model's own derived grain: clock declared, no top-level identity.
    assert!(
        report.contains("derived grain: partition"),
        "daily_events_status declares only a clock at the top level: {report}"
    );

    // Both source refs appear as inbound edges, labelled `(source)`.
    assert!(
        report.contains("sources.raw.events (source)"),
        "expected the raw.events source edge: {report}"
    );
    assert!(
        report.contains("sources.raw.user_status (source)"),
        "expected the raw.user_status source edge: {report}"
    );

    // raw.events: identity declared (event_id), no clock -> keyed-dimension.
    // raw.user_status: both declared, partition_column not in key -> keyed.
    // Both source edges use the same field names as the model's own
    // contract row set.
    let clock_rows = report.matches("clock:").count();
    let identity_rows = report.matches("identity:").count();
    let grain_rows = report.matches("derived grain:").count();
    assert_eq!(
        clock_rows, 3,
        "expected 3 clock rows (own + 2 source edges): {report}"
    );
    assert_eq!(
        identity_rows, 3,
        "expected 3 identity rows (own + 2 source edges): {report}"
    );
    assert_eq!(
        grain_rows, 3,
        "expected 3 derived-grain rows (own + 2 source edges): {report}"
    );
    assert!(
        report.contains("identity: event_id"),
        "expected raw.events's declared identity to render: {report}"
    );
    assert!(
        report.contains("identity: user_id"),
        "expected raw.user_status's declared identity to render: {report}"
    );
}

/// `user_spend_running_total` (already exercised above for its route-1
/// locality) also demonstrates a **model** edge's contract rendering through
/// the same field names a source edge uses — `(model)` label, `clock:`,
/// `identity:`, `derived grain:` rows for the composed upstream
/// `user_daily_spend`.
#[test]
fn explain_prints_relation_contract_for_model_edge() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");

    let report = build_report_for(&project_dir, "user_spend_running_total")
        .expect("model has a maintenance plan");

    assert!(
        report.contains("user_daily_spend (model)"),
        "expected the composed upstream rendered as a model-provider edge: {report}"
    );
    assert!(
        report.contains("Relation contract:"),
        "expected the model's own contract section: {report}"
    );
    // Same field names as the source-edge case above.
    assert!(report.contains("clock:"), "{report}");
    assert!(report.contains("identity:"), "{report}");
    assert!(report.contains("derived grain:"), "{report}");
}

/// JSON leg: `smelt explain <model> --show-sql --json` carries the
/// `contract` object (own fill) and `inbound_edges[].contract` (per-edge
/// fill) with identical field paths (`clock`, `identity`, `derived_grain`)
/// for both a source-provider and a model-provider edge.
#[test]
fn json_carries_relation_contract_for_both_providers() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_smelt"))
        .arg("explain")
        .arg("daily_events_status")
        .arg("--show-sql")
        .arg("--json")
        .arg("--project-dir")
        .arg(&project_dir)
        .output()
        .expect("spawn smelt explain daily_events_status --show-sql --json");

    assert!(
        output.status.success(),
        "smelt explain --json failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("invalid JSON: {e}: {stdout}"));

    let own_contract = parsed
        .get("contract")
        .unwrap_or_else(|| panic!("expected a top-level 'contract' object: {stdout}"));
    assert!(
        own_contract.get("clock").is_some(),
        "expected the model's own contract to carry a 'clock' field path: {stdout}"
    );
    assert!(
        own_contract.get("derived_grain").is_some(),
        "expected the model's own contract to carry a 'derived_grain' field path: {stdout}"
    );

    let edges = parsed
        .get("inbound_edges")
        .and_then(|e| e.as_array())
        .unwrap_or_else(|| panic!("expected a top-level 'inbound_edges' array: {stdout}"));
    assert_eq!(edges.len(), 2, "expected both source edges: {stdout}");
    // `raw.events` declares identity only (no clock) — keyed-dimension;
    // `raw.user_status` declares both — keyed, time-partitioned. Both
    // contracts use the same field *names* (`clock`, `identity`,
    // `derived_grain`) as the model's own contract object above; which
    // fields are present differs per provider's own declared facts, exactly
    // as it does for the model's own contract.
    let mut names: Vec<String> = Vec::new();
    for edge in edges {
        assert_eq!(
            edge.get("provider").and_then(|p| p.as_str()),
            Some("source"),
            "expected both edges to be source-provided: {edge}"
        );
        let contract = edge
            .get("contract")
            .unwrap_or_else(|| panic!("expected an edge 'contract' object: {edge}"));
        assert!(
            contract.get("derived_grain").is_some(),
            "expected every edge contract to carry a 'derived_grain' field path: {edge}"
        );
        names.push(
            edge.get("name")
                .and_then(|n| n.as_str())
                .unwrap_or_default()
                .to_string(),
        );
    }
    assert!(names.contains(&"sources.raw.events".to_string()));
    assert!(names.contains(&"sources.raw.user_status".to_string()));

    let events_edge = edges
        .iter()
        .find(|e| e.get("name").and_then(|n| n.as_str()) == Some("sources.raw.events"))
        .expect("raw.events edge present");
    assert!(
        events_edge["contract"].get("clock").is_none(),
        "raw.events declares no clock: {events_edge}"
    );
    assert_eq!(
        events_edge["contract"]["identity"],
        serde_json::json!(["event_id"]),
        "expected raw.events's declared identity: {events_edge}"
    );

    let user_status_edge = edges
        .iter()
        .find(|e| e.get("name").and_then(|n| n.as_str()) == Some("sources.raw.user_status"))
        .expect("raw.user_status edge present");
    assert!(
        user_status_edge["contract"].get("clock").is_some(),
        "expected raw.user_status's declared clock, using the same 'clock' field path as \
         the model's own contract: {user_status_edge}"
    );
}

// ---------------------------------------------------------------------------
// Region row identity (P2, `docs/specs/model_properties.md` §"Region row
// identity") — `smelt explain` prints each cell's row identity alongside its
// technique (`docs/plans/20260715-composed-axes-conditional-maintenance.md`
// Phase C3).
// ---------------------------------------------------------------------------

/// `user_daily_spend` (`examples/timeseries`) is a `grain: key` model whose
/// key is the outermost `GROUP BY user_id, spend_date` — no top-level
/// `unique_key:` is written, so the row identity is the walk's own proven
/// grain key, printed right alongside the cell's technique line.
#[test]
fn explain_prints_key_row_identity_for_a_group_by_keyed_model() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");

    let report =
        build_report_for(&project_dir, "user_daily_spend").expect("model has a maintenance plan");

    assert!(
        report.contains("technique:"),
        "expected a technique line to anchor the row-identity line against: {report}"
    );
    assert!(
        report.contains("region key: Key"),
        "expected a Key(...) region key for the GROUP BY-keyed output: {report}"
    );
    assert!(
        report.contains("user_id") && report.contains("spend_date"),
        "expected the proven grain key's own columns named in the report: {report}"
    );
}

/// `daily_events_status` (`examples/timeseries`) is a `grain: partition`
/// model with no top-level `unique_key:` and no `GROUP BY` — no key can be
/// established, so every cell's row identity falls back to the
/// identity-free `WholeRow` multiset diff.
#[test]
fn explain_prints_whole_row_identity_for_a_keyless_partition_model() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");

    let report = build_report_for(&project_dir, "daily_events_status")
        .expect("model has a maintenance plan");

    assert!(
        report.contains("region key: WholeRow"),
        "expected the keyless partition-grain fallback to be WholeRow: {report}"
    );
}

/// Phase R1 (`docs/plans/20260715-composed-axes-conditional-maintenance.md`):
/// `smelt explain <model>` prints, per cell, the open write-pattern
/// registry's admissible pattern-name set and the active `write:` pin (if
/// any) — `docs/specs/incremental_models.md` §"Per-cell write addressing".
/// Built as a self-contained tempdir workspace (not one of the shared
/// `examples/` fixtures) so the `write:` pin doesn't perturb any other
/// example-based test or the `example_diagnostics`/LSP parity gates.
mod write_pin_explain_surface {
    use super::build_report_for;
    use std::fs;

    const SMELT_YML: &str = r#"
name: write_pin_explain_fixture
version: 1

paths:
  - models

targets:
  dev:
    type: duckdb
    database: target/dev.duckdb
    schema: main

default_materialization: view
"#;

    /// A `grain: partition` model (no `unique_key:`, so identity-free) with
    /// a `write: region` pin on its backfill cell. `region` requires only
    /// the declared partition axis this output has, so it resolves cleanly
    /// — the report must show it as the active pin and list it inside the
    /// admissible set.
    const MODEL_WITH_VALID_PIN: &str = r#"---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: d
  partition_column: d
  granularity: day
maintenance:
  cells:
    - columns: [amount]
      on: backfill
      write: region
---
SELECT d, amount FROM smelt.sources.payments
"#;

    const PAYMENTS_SOURCE: &str = r#"
columns:
  - { name: d, type: DATE, nullable: false }
  - { name: amount, type: DOUBLE, nullable: false }
"#;

    #[test]
    fn explain_prints_admissible_set_and_active_pin() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        fs::write(root.join("smelt.yml"), SMELT_YML).unwrap();
        fs::create_dir_all(root.join("models")).unwrap();
        fs::create_dir_all(root.join("models/sources")).unwrap();
        fs::write(root.join("models/revenue.sql"), MODEL_WITH_VALID_PIN).unwrap();
        fs::write(root.join("models/sources/payments.yml"), PAYMENTS_SOURCE).unwrap();

        let report = build_report_for(root, "revenue").expect("revenue has a maintenance plan");

        assert!(
            report.contains("admissible write patterns:"),
            "expected an admissible-write-patterns row per cell: {report}"
        );
        assert!(
            report.contains("write pin: region"),
            "expected the active `write: region` pin to be printed: {report}"
        );
        // The admissible set must actually list the pinned pattern — a pin
        // never widens the set, and here it should already be a member.
        let admissible_line = report
            .lines()
            .find(|l| l.contains("admissible write patterns:"))
            .expect("admissible-set line present");
        assert!(
            admissible_line.contains("region"),
            "the admissible set must include the pinned pattern 'region': {admissible_line}"
        );
    }

    /// A model that declares no `maintenance.cells[]` at all still prints
    /// the admissible-set row per cell, with `write pin: (none)` — the row
    /// is unconditional, not only shown when a pin exists.
    #[test]
    fn explain_prints_none_pin_when_no_cells_declared() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let model_no_pin = r#"---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: d
  partition_column: d
  granularity: day
---
SELECT d, amount FROM smelt.sources.payments
"#;
        fs::write(root.join("smelt.yml"), SMELT_YML).unwrap();
        fs::create_dir_all(root.join("models")).unwrap();
        fs::create_dir_all(root.join("models/sources")).unwrap();
        fs::write(root.join("models/revenue.sql"), model_no_pin).unwrap();
        fs::write(root.join("models/sources/payments.yml"), PAYMENTS_SOURCE).unwrap();

        let report = build_report_for(root, "revenue").expect("revenue has a maintenance plan");

        assert!(
            report.contains("write pin: (none)"),
            "expected an unconditional 'write pin: (none)' row absent any cells[] entry: {report}"
        );
    }
}

// ---------------------------------------------------------------------------
// Observed-delta recording + projection surface
// (`docs/plans/20260715-composed-axes-conditional-maintenance.md` Phase D4;
// `docs/specs/incremental_models.md` §"The graph layer" — "Observed deltas
// on model edges", §"What the composed shape uniquely enables" — "Exact
// key→partition dirt projection").
//
// `examples/timeseries/models/daily_events_enriched.sql` USED to be the
// real fixture exercising a `Technique::ColumnScopedMerge` cell (a
// single-input mutable dimension enrichment) — the only technique family D2
// wired observed-delta recording for. As of `docs/plans/
// 20260808-membership-sensitivity.md` Phase 1, `raw.users` being read in
// that model's `JOIN`'s own `ON` predicate makes it membership-sensitive
// instead (`Technique::DeleteInsert`), so NO fixture in this workspace
// reaches `ColumnScopedMerge` anymore (Phase 2's own reachability verdict);
// the recording-status test below is now built over a synthetic
// `MaintenancePlan`, not a real fixture — see its own doc comment. The
// projection-form assertion still uses real fixtures (route 1's
// `user_daily_spend`, route 3's `silver.events_deduped`), unaffected by
// this change.
///
/// **Post-`docs/plans/20260808-membership-sensitivity.md` Phase 3 note:**
/// `daily_events_enriched` (the real fixture) no longer derives ANY
/// `Technique::ColumnScopedMerge` cell at all — `raw.users` is read in the
/// enrichment `JOIN`'s own `ON` predicate, a row-admission read, which
/// makes EVERY column group's cell for that trigger membership-sensitive
/// (`Technique::DeleteInsert`), never `ColumnScopedMerge` (Phase 1's review
/// checklist: "membership cells cannot receive ColumnScopedMerge"). Per
/// Phase 2's own reachability verdict, no fixture in this workspace reaches
/// `ColumnScopedMerge` anymore — so this test (which exists to check the
/// EXPLAIN PRINTING logic for a `ColumnScopedMerge` cell with `WholeRow` row
/// identity, `crates/smelt-cli/src/explain.rs` lines ~353-364) is rewritten
/// to build its `MaintenancePlan` synthetically, mirroring
/// `write_variant_explain_surface`'s own pattern below — the printing logic
/// is independent of whether real SQL derivation can currently produce this
/// shape, and constructing a fictitious SQL fixture to keep the technique
/// artificially reachable would misrepresent what the derivation actually
/// admits today.
#[test]
fn explain_prints_observed_delta_recording_status_for_a_conditional_cell() {
    use std::collections::BTreeSet;

    use smelt_cli::explain::RelationContractView;
    use smelt_db::queries::maintenance::MaintenancePlanResult;
    use smelt_logical::maintenance::{
        ColumnGroup, Corner, MaintenancePlan, PartitionLocal, PlanCell, RowIdentity,
        RowIdentityVerdict, Technique, Trigger,
    };

    let cell = PlanCell {
        group: "{user_name}".to_string(),
        trigger: Trigger::UpstreamMutation {
            source: "raw.users".to_string(),
        },
        corner: Corner::ColumnMerge,
        technique: Technique::ColumnScopedMerge,
        partition_local: PartitionLocal::Yes,
        scans: vec![],
        ledger_catch_up: false,
        row_identity: RowIdentityVerdict {
            identity: RowIdentity::WholeRow,
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
            columns: vec!["user_name".to_string()],
            mutation_sensitivity: Default::default(),
            membership_sensitivity: BTreeSet::new(),
        }],
        degenerate: vec![],
        state_columns: vec![],
        execution_postures: None,
        is_snapshot_reconcile: None,
        comparability: vec![],
        succession_advisories: vec![],
    };
    let __profile = synthetic_profile(&result, "daily_events_enriched");
    let report = build_maintenance_plan_report(
        "daily_events_enriched",
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
        &__profile,
    )
    .expect("build_maintenance_plan_report");

    assert_eq!(
        report
            .matches("observed-delta recording: yes (change-suppressed column-scoped MERGE)")
            .count(),
        0,
        "a ColumnScopedMerge cell with WholeRow row identity must never claim recording: yes: \
         {report}"
    );
    assert_eq!(
        report.matches("observed-delta recording: no").count(),
        1,
        "expected the negative recording row exactly once, on the model's one \
         ColumnScopedMerge cell: {report}"
    );
}

/// A `ColumnScopedMerge` cell whose P2 row identity resolves `WholeRow`
/// must print "no" for observed-delta recording, never "yes" — a
/// `WholeRow` cell has no per-row join identity to compare on, so
/// `choice::resolve_write_suppression` always fail-closes to
/// `Unconditional` for it (`crates/smelt-logical/src/maintenance/
/// choice.rs`'s `whole_row_identity_refuses_regardless_of_comparability`
/// unit test covers the same fail-closed rule at the derivation layer; this
/// covers the `smelt explain` reporting surface). The plan carries a
/// SIBLING `Technique::DeleteInsert` cell alongside the `ColumnScopedMerge`
/// cell under test, proving the "no" line is correctly isolated to the
/// `ColumnScopedMerge` cell's own block and never leaks onto (or is
/// swallowed by) an unrelated sibling cell's report lines.
///
/// **Post-`docs/plans/20260808-membership-sensitivity.md` Phase 3 note:**
/// originally built over a real fact+dimension enrichment fixture (mirroring
/// `examples/timeseries/models/daily_events_enriched.sql`); rewritten to a
/// synthetic `MaintenancePlan` for the same reason as
/// `explain_prints_observed_delta_recording_status_for_a_conditional_cell`
/// above — no fixture in this workspace derives a `ColumnScopedMerge` cell
/// anymore (Phase 1's membership-sensitivity derivation), so the EXPLAIN
/// PRINTING logic under test needs a hand-built plan to reach it at all.
#[test]
fn explain_prints_no_recording_for_a_whole_row_identity_conditional_cell() {
    use std::collections::BTreeSet;

    use smelt_cli::explain::RelationContractView;
    use smelt_db::queries::maintenance::MaintenancePlanResult;
    use smelt_logical::maintenance::{
        ColumnGroup, Corner, MaintenancePlan, PartitionLocal, PlanCell, RowIdentity,
        RowIdentityVerdict, Technique, Trigger,
    };

    let merge_cell = PlanCell {
        group: "{user_name}".to_string(),
        trigger: Trigger::UpstreamMutation {
            source: "users".to_string(),
        },
        corner: Corner::ColumnMerge,
        technique: Technique::ColumnScopedMerge,
        partition_local: PartitionLocal::Yes,
        scans: vec![],
        ledger_catch_up: false,
        row_identity: RowIdentityVerdict {
            identity: RowIdentity::WholeRow,
            proven_mismatch: None,
        },
        skeleton_source_closure: None,
        fingerprint_projections: Default::default(),
        key_scope: None,
        state_downgrade: None,
    };
    let sibling_cell = PlanCell {
        group: "{event_type, user_id}".to_string(),
        trigger: Trigger::UpstreamMutation {
            source: "users".to_string(),
        },
        corner: Corner::RecomputeRegion,
        technique: Technique::DeleteInsert,
        partition_local: PartitionLocal::No {
            source: "users".to_string(),
            why: "unclocked source is read in full on every recompute".to_string(),
        },
        scans: vec![],
        ledger_catch_up: false,
        row_identity: RowIdentityVerdict {
            identity: RowIdentity::WholeRow,
            proven_mismatch: None,
        },
        skeleton_source_closure: None,
        fingerprint_projections: Default::default(),
        key_scope: None,
        state_downgrade: None,
    };
    let result = MaintenancePlanResult {
        plan: MaintenancePlan {
            cells: vec![merge_cell, sibling_cell],
            refusals: vec![],
            key_locality: None,
        },
        column_groups: vec![
            ColumnGroup {
                columns: vec!["user_name".to_string()],
                mutation_sensitivity: Default::default(),
                membership_sensitivity: BTreeSet::new(),
            },
            ColumnGroup {
                columns: vec!["event_type".to_string(), "user_id".to_string()],
                mutation_sensitivity: Default::default(),
                membership_sensitivity: BTreeSet::new(),
            },
        ],
        degenerate: vec![],
        state_columns: vec![],
        execution_postures: None,
        is_snapshot_reconcile: None,
        comparability: vec![],
        succession_advisories: vec![],
    };
    let __profile = synthetic_profile(&result, "events_enriched");
    let report = build_maintenance_plan_report(
        "events_enriched",
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
        &__profile,
    )
    .expect("build_maintenance_plan_report");

    assert!(
        report.contains("region key: WholeRow"),
        "fixture must actually exercise the WholeRow identity case: {report}"
    );
    // Each cell's block starts at its own "  - group ..." header line; split
    // on that marker to isolate the ColumnScopedMerge cell's own lines from
    // the sibling DeleteInsert cell.
    let cell_block = report
        .split("  - group ")
        .find(|block| block.contains("technique: ColumnScopedMerge"))
        .expect("expected the admitted ColumnScopedMerge cell");
    assert!(
        cell_block.contains("observed-delta recording: no"),
        "a WholeRow-identity ColumnScopedMerge cell must never claim recording: yes: {cell_block}\n\nfull report:\n{report}"
    );
    assert!(
        !cell_block.contains("observed-delta recording: yes"),
        "a WholeRow-identity ColumnScopedMerge cell must never claim recording: yes: {cell_block}"
    );
    let sibling_block = report
        .split("  - group ")
        .find(|block| block.contains("technique: DeleteInsert"))
        .expect("expected the sibling DeleteInsert cell");
    assert!(
        !sibling_block.contains("observed-delta recording"),
        "a DeleteInsert cell must never print an observed-delta recording line at all — that \
         reporting family is wired only for ColumnScopedMerge: {sibling_block}"
    );
}

/// A composed model under locality route 1 (key-embedded) reports an
/// *exact* observed-delta projection — no widening, since a stored row's
/// partition value is a per-key constant under this route.
#[test]
fn explain_prints_exact_projection_for_a_route_one_composed_model() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");

    let report =
        build_report_for(&project_dir, "user_daily_spend").expect("model has a maintenance plan");

    assert!(
        report.contains("observed-delta projection: exact (key-embedded)"),
        "expected an exact projection line for route 1: {report}"
    );
}

/// A composed model under locality route 3 (recurrence-bounded) reports a
/// *widened* observed-delta projection — a key's partition value may move
/// under this route, so the projected dirt widens backward by `r` plus the
/// route's own margins (`silver.events_deduped`, the flagship composed
/// dedupe fixture).
#[test]
fn explain_prints_widened_projection_for_a_route_three_composed_model() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/web_analytics")
        .canonicalize()
        .expect("examples/web_analytics exists");

    let report = build_report_for(&project_dir, "silver.events_deduped")
        .expect("model has a maintenance plan");

    assert!(
        report.contains("observed-delta projection: widened by `r` + margins"),
        "expected a widened projection line for route 3: {report}"
    );
}

/// A bare keyed model (identity, no established key temporal locality) has
/// no partition axis to project observed deltas onto at all — the report
/// must show no projection row, distinct from a composed model's exact or
/// widened form.
#[test]
fn explain_shows_no_projection_row_for_a_bare_keyed_model() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/web_analytics")
        .canonicalize()
        .expect("examples/web_analytics exists");

    let report = build_report_for(&project_dir, "silver.device_user_edges")
        .expect("model has a maintenance plan");

    assert!(
        !report.contains("Key temporal locality:"),
        "silver.device_user_edges is bare keyed — no locality section expected: {report}"
    );
    assert!(
        !report.contains("observed-delta projection:"),
        "a bare keyed model must print no projection row at all: {report}"
    );
}

// ---------------------------------------------------------------------------
// Write variant (`docs/plans/20260715-composed-axes-conditional-
// maintenance.md` Phase G1; `docs/specs/incremental_models.md` §"Windowed
// maintenance and the horizon" category 2, §"Interchangeability and
// choice"): `smelt explain` shows which matched-arm shape a suppressible
// cell's conditional-variant dimension resolves to, and why. Real fixtures
// today only ever derive a steady-state trigger for `ColumnScopedMerge`/
// `KeyedFold` cells (`derive_backfill` always emits `Technique::DeleteInsert`
// for `Trigger::Backfill`, and `Trigger::ColumnAdded` cells are only
// constructed from an explicit `ModelDiff` a plain `smelt explain` never
// supplies) — so the first-build-posture branch is exercised directly
// against `build_maintenance_plan_report` with a hand-built
// `MaintenancePlanResult`, the same way `crates/smelt-runtime/tests/
// technique_lowering.rs` hand-builds `PlanCell`s to reach shapes no real
// fixture derives yet.
// ---------------------------------------------------------------------------

mod write_variant_explain_surface {
    use std::collections::BTreeSet;

    use super::synthetic_profile;
    use smelt_cli::build_maintenance_plan_report;
    use smelt_cli::explain::RelationContractView;
    use smelt_db::queries::maintenance::MaintenancePlanResult;
    use smelt_logical::analysis::walk::{ColumnComparability, Comparability};
    use smelt_logical::maintenance::{
        Corner, MaintenancePlan, PartitionLocal, PlanCell, RowIdentity, RowIdentityVerdict,
        Technique, Trigger,
    };

    /// `{tier}`, proven `Comparable` — the P3 half of the write-suppression
    /// proof, threaded here so `report_for`'s cells (all `Key`-identity,
    /// `ColumnScopedMerge`) reach the SAME "admitted, preference/pin
    /// decides" branches these tests exercised before `smelt explain`
    /// consulted real comparability instead of a `facts.has_identity`-only
    /// proxy. `technique_suppress_pin_on_an_incomparable_column_is_a_hard_
    /// refusal` below is the one test in this module that deliberately
    /// supplies a DIFFERENT (`Incomparable`) vector instead.
    fn comparable_tier() -> Vec<ColumnComparability> {
        vec![ColumnComparability {
            output: "tier".to_string(),
            comparability: Comparability::Comparable,
        }]
    }

    fn key_identity() -> RowIdentityVerdict {
        RowIdentityVerdict {
            identity: RowIdentity::Key(vec!["user_id".to_string()]),
            proven_mismatch: None,
        }
    }

    fn base_cell(trigger: Trigger, ledger_catch_up: bool) -> PlanCell {
        PlanCell {
            group: "{tier}".to_string(),
            trigger,
            corner: Corner::ColumnMerge,
            technique: Technique::ColumnScopedMerge,
            partition_local: PartitionLocal::Yes,
            scans: vec![],
            ledger_catch_up,
            row_identity: key_identity(),
            skeleton_source_closure: None,
            fingerprint_projections: Default::default(),
            key_scope: None,
            state_downgrade: None,
        }
    }

    fn report_for(cell: PlanCell) -> String {
        report_for_with_overrides(cell, &[], None, comparable_tier())
            .expect("build_maintenance_plan_report")
    }

    fn report_for_with_overrides(
        cell: PlanCell,
        cells_cfg: &[smelt_core::config::MaintenanceCellConfig],
        defaults_cfg: Option<&smelt_core::config::MaintenanceDefaults>,
        comparability: Vec<ColumnComparability>,
    ) -> anyhow::Result<String> {
        use smelt_logical::maintenance::ColumnGroup;

        let result = MaintenancePlanResult {
            plan: MaintenancePlan {
                cells: vec![cell],
                refusals: vec![],
                key_locality: None,
            },
            // `base_cell`'s group is `{tier}` (`ColumnGroup::name()` derives
            // the display name from `columns`), matching the single column
            // this fixture's pin tests target.
            column_groups: vec![ColumnGroup {
                columns: vec!["tier".to_string()],
                mutation_sensitivity: Default::default(),
                membership_sensitivity: BTreeSet::new(),
            }],
            degenerate: vec![],
            state_columns: vec![],
            execution_postures: None,
            is_snapshot_reconcile: None,
            comparability,
            succession_advisories: vec![],
        };
        let profile = synthetic_profile(&result, "write_variant_fixture");
        build_maintenance_plan_report(
            "write_variant_fixture",
            &result,
            &RelationContractView::from_facts(None, None),
            &[],
            cells_cfg,
            defaults_cfg,
            None,
            &[],
            &[],
            smelt_core::config::ProbeCadence::PerRun,
            &[],
            None,
            None,
            &profile,
        )
    }

    /// A steady-state trigger (`Trigger::UpstreamMutation`, no ledger
    /// catch-up) over a proven `Key` row identity prefers the
    /// change-suppressed matched arm.
    #[test]
    fn steady_state_trigger_prefers_suppressed() {
        let cell = base_cell(
            Trigger::UpstreamMutation {
                source: "sources.users".to_string(),
            },
            false,
        );
        let report = report_for(cell);
        assert!(
            report.contains("write variant: suppressed (preference"),
            "expected the steady-state trigger to prefer the suppressed matched arm: {report}"
        );
    }

    /// A definition-change backfill cell (`ledger_catch_up: true`) is
    /// admitted but not preferred — first-build posture — even over the
    /// same proven `Key` row identity, and even on an otherwise
    /// steady-state trigger kind.
    #[test]
    fn ledger_catch_up_cell_shows_first_build_posture() {
        let cell = base_cell(
            Trigger::UpstreamMutation {
                source: "sources.users".to_string(),
            },
            true,
        );
        let report = report_for(cell);
        assert!(
            report.contains("write variant: unconditional (first-build posture"),
            "expected a definition-change backfill cell to show the first-build posture, not \
             the steady-state preference: {report}"
        );
    }

    /// No proven row identity (`WholeRow`) never admits the conditional
    /// variant at all — the report must show the default, never the
    /// preference or first-build lines.
    #[test]
    fn whole_row_identity_shows_default_not_admitted() {
        let mut cell = base_cell(
            Trigger::UpstreamMutation {
                source: "sources.users".to_string(),
            },
            false,
        );
        cell.row_identity = RowIdentityVerdict {
            identity: RowIdentity::WholeRow,
            proven_mismatch: None,
        };
        let report = report_for(cell);
        assert!(
            report.contains("write variant: unconditional (not admitted"),
            "expected the no-proven-identity default line, never a preference/first-build \
             claim: {report}"
        );
        assert!(!report.contains("write variant: suppressed"));
    }

    fn cell_cfg_with_technique(
        on: &str,
        technique: smelt_core::config::CellTechnique,
    ) -> smelt_core::config::MaintenanceCellConfig {
        smelt_core::config::MaintenanceCellConfig {
            columns: vec!["tier".to_string()],
            on: on.to_string(),
            prefer: None,
            technique: Some(technique),
            write: None,
        }
    }

    /// A `technique: suppress` pin forces the change-suppressed matched arm
    /// on for a first-build/definition-change-backfill cell that would
    /// otherwise default to unconditional (`ledger_catch_up_cell_shows_
    /// first_build_posture` above, absent a pin).
    #[test]
    fn technique_suppress_pin_shows_suppressed_even_on_first_build_posture() {
        let cell = base_cell(
            Trigger::UpstreamMutation {
                source: "sources.users".to_string(),
            },
            true,
        );
        let cells_cfg = vec![cell_cfg_with_technique(
            "sources.users",
            smelt_core::config::CellTechnique::Suppress,
        )];
        let report = report_for_with_overrides(cell, &cells_cfg, None, comparable_tier())
            .expect("build_maintenance_plan_report");
        assert!(
            report.contains("write variant: suppressed (pinned via `technique: suppress`"),
            "expected the pin to override the first-build-posture default: {report}"
        );
    }

    /// A `technique: unconditional` pin forces the plain matched arm on a
    /// steady-state cell that would otherwise prefer suppression
    /// (`steady_state_trigger_prefers_suppressed` above, absent a pin).
    #[test]
    fn technique_unconditional_pin_shows_unconditional_even_on_steady_state_preference() {
        let cell = base_cell(
            Trigger::UpstreamMutation {
                source: "sources.users".to_string(),
            },
            false,
        );
        let cells_cfg = vec![cell_cfg_with_technique(
            "sources.users",
            smelt_core::config::CellTechnique::Unconditional,
        )];
        let report = report_for_with_overrides(cell, &cells_cfg, None, comparable_tier())
            .expect("build_maintenance_plan_report");
        assert!(
            report.contains("write variant: unconditional (pinned via `technique: unconditional`"),
            "expected the pin to override the steady-state preference: {report}"
        );
    }

    /// A `technique: suppress` pin over a cell whose write-suppression proof
    /// genuinely refuses (P2: no proven row identity, `WholeRow`) is a hard
    /// `ChoiceRefusal` — `smelt explain` must propagate it as a real error,
    /// never a silently-wrong "suppressed" or "falls back to unconditional"
    /// success line (the self-contradictory/silent-success text this test
    /// replaces coverage for).
    #[test]
    fn technique_suppress_pin_on_whole_row_identity_is_a_hard_refusal() {
        // `RowIdentity::WholeRow` (no proven row identity) always resolves
        // `resolve_write_suppression` to `Unconditional` — the P2 check
        // short-circuits before comparability or the column group are even
        // consulted, so a `technique: suppress` pin over this cell is
        // genuinely, always inadmissible. `smelt explain` must propagate
        // that `ChoiceRefusal` as a real error, never a silently-wrong
        // "suppressed" or "falls back to unconditional" success line (the
        // self-contradictory/silent-success text this test replaces
        // coverage for).
        let mut cell = base_cell(
            Trigger::UpstreamMutation {
                source: "sources.users".to_string(),
            },
            false,
        );
        cell.row_identity = RowIdentityVerdict {
            identity: RowIdentity::WholeRow,
            proven_mismatch: None,
        };
        let cells_cfg = vec![cell_cfg_with_technique(
            "sources.users",
            smelt_core::config::CellTechnique::Suppress,
        )];
        let err = report_for_with_overrides(cell, &cells_cfg, None, comparable_tier()).expect_err(
            "an inadmissible `technique: suppress` pin must refuse, never print a \
             success/fallback line",
        );
        let message = err.to_string();
        assert!(
            message.contains("technique: suppress"),
            "expected the refusal to name the pin that could not be honoured: {message}"
        );
    }

    /// A `technique: suppress` pin over a cell that DOES carry a proven
    /// `Key` row identity (P2 holds) but whose compared column is not
    /// proven comparable across runs (P3 fails) is the same hard
    /// `ChoiceRefusal` as the `WholeRow` case above — `smelt explain` must
    /// propagate it too, not only the P2-decidable case
    /// (`incremental_models.md` §"Per-cell write addressing" → "User
    /// pins").
    #[test]
    fn technique_suppress_pin_on_an_incomparable_column_is_a_hard_refusal() {
        let cell = base_cell(
            Trigger::UpstreamMutation {
                source: "sources.users".to_string(),
            },
            false,
        );
        let cells_cfg = vec![cell_cfg_with_technique(
            "sources.users",
            smelt_core::config::CellTechnique::Suppress,
        )];
        let incomparable_tier = vec![ColumnComparability {
            output: "tier".to_string(),
            comparability: Comparability::Incomparable,
        }];
        let err = report_for_with_overrides(cell, &cells_cfg, None, incomparable_tier).expect_err(
            "a `technique: suppress` pin over an incomparable compared column must refuse, \
             never print a success/fallback line",
        );
        let message = err.to_string();
        assert!(
            message.contains("technique: suppress"),
            "expected the refusal to name the pin that could not be honoured: {message}"
        );
        assert!(
            message.contains("tier"),
            "expected the refusal to trace back to the incomparable column: {message}"
        );
    }
}

/// `smelt explain <model> --json` WITHOUT `--show-sql` must emit the same
/// per-model JSON report, not silently fall back to the plain-text rendering
/// (`docs/specs/cli.md` §"`smelt explain <model>` maintenance-plan report":
/// "`--json` is honored with a model-name argument — with or without
/// `--show-sql`"; fail-loud discipline — a recognised flag is never silently
/// ignored). Before the fix, the `!show_sql` early return printed the text
/// report and a machine consumer got prose with exit 0.
#[test]
fn json_without_show_sql_emits_json() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_smelt"))
        .arg("explain")
        .arg("user_spend_rollup")
        .arg("--json")
        .arg("--project-dir")
        .arg(&project_dir)
        .output()
        .expect("spawn smelt explain user_spend_rollup --json");

    assert!(
        output.status.success(),
        "smelt explain --json failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("--json without --show-sql must emit JSON, got: {e}\n{stdout}"));

    let cells = json["cells"]
        .as_array()
        .unwrap_or_else(|| panic!("expected a `cells` array: {stdout}"));
    assert!(
        !cells.is_empty(),
        "expected at least one plan cell for user_spend_rollup: {stdout}"
    );
    // The JSON form always carries the preview arrays — symbolic
    // `{{window_start}}`/`{{window_end}}` statements included — regardless
    // of `--show-sql` (spec: "the same schema either way").
    assert!(
        cells[0]["technique_previews"].is_array(),
        "expected technique_previews per cell: {stdout}"
    );
    assert!(
        cells[0]["statements"].is_array(),
        "expected statements per cell: {stdout}"
    );
}
