use std::path::Path;

use crate::support::build_report_for;

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
