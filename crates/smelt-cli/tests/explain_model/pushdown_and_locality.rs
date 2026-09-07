use std::path::Path;

use crate::support::build_report_for;

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
