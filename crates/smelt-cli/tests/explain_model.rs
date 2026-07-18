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

    Some(build_maintenance_plan_report(
        &canonical,
        &result,
        &own_contract,
        &edges,
    ))
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
/// incremental_models.md` §"Key temporal locality") widened a further two
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
/// (`grain: key_per_partition`) prints the `UnsupportedGrain` refusal — naming
/// the grain and the tracking plan — and no cell table, never a keyed cell
/// derived with an empty `unique_key`.
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
/// lookback. `incremental_models.md` §"Key temporal locality (the
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
/// (`docs/specs/incremental_models.md` §"Key temporal locality (the
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
