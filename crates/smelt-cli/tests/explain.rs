//! TDD tests for Phase 3 of `docs/plans/20260725-ui-model-diagnostics.md`
//! ("`smelt-cli` — thin `explain.rs` + `--technique` flag"). Oracle:
//! `docs/specs/ui_model_diagnostics.md` §Surface "CLI"; §Semantics
//! "Thin-consumer boundary".
//!
//! `explain.rs`'s report builders now call
//! `smelt_runtime::diagnostics::build_model_diagnostics` (whenever `--json`
//! or `--technique` is given) and render from the returned
//! `ModelDiagnostics`, rather than deriving contract/statement data
//! themselves. The default `smelt explain <model> --show-sql` text report
//! (no `--technique`) must remain byte-identical to its pre-refactor output
//! — the golden fixture below pins that.

use std::path::Path;
use std::process::Command;

fn timeseries_project_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists")
}

fn run_explain(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_smelt"))
        .arg("explain")
        .args(args)
        .output()
        .expect("spawn smelt explain")
}

/// Golden/snapshot comparison: `smelt explain daily_events --show-sql`
/// (no `--technique`) must be byte-identical to the fixture captured before
/// this phase's refactor — the thin-consumer boundary's main risk is that
/// routing the default report through the shared `smelt-runtime::
/// diagnostics` builder accidentally changes existing output.
#[test]
fn show_sql_output_unchanged() {
    let project_dir = timeseries_project_dir();
    let output = run_explain(&[
        "daily_events",
        "--show-sql",
        "--project-dir",
        project_dir.to_str().expect("utf8 path"),
    ]);
    assert!(
        output.status.success(),
        "smelt explain daily_events --show-sql failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);

    let golden = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/explain_show_sql_daily_events_golden.txt"),
    )
    .expect("read golden fixture");

    assert_eq!(
        stdout, golden,
        "default `--show-sql` output (no --technique) must be byte-identical to the \
         pre-refactor golden fixture — the thin-consumer boundary's main risk \
         (docs/specs/ui_model_diagnostics.md §Semantics \"Thin-consumer boundary\")"
    );
}

/// `--show-sql --technique keyed_fold` on `user_daily_spend` (`grain: key`
/// and `timeseries:`, admitted `Technique::KeyedFold`) must render that
/// technique's own preview statements — the `MERGE INTO` fold, not the
/// default admitted-technique rendering — for every cell that has a
/// preview for it.
#[test]
fn technique_flag_renders_named_technique() {
    let project_dir = timeseries_project_dir();
    let output = run_explain(&[
        "user_daily_spend",
        "--show-sql",
        "--technique",
        "keyed_fold",
        "--project-dir",
        project_dir.to_str().expect("utf8 path"),
    ]);
    assert!(
        output.status.success(),
        "smelt explain user_daily_spend --show-sql --technique keyed_fold failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        stdout.contains("technique preview: KeyedFold"),
        "expected a KeyedFold technique-preview header per cell: {stdout}"
    );
    assert!(
        stdout.contains("verdict: Admitted"),
        "user_daily_spend's own admitted technique is KeyedFold, so its preview entry must \
         carry the Admitted verdict: {stdout}"
    );
    assert!(
        stdout.contains("MERGE INTO"),
        "expected the keyed-fold MERGE statement in the technique-preview output: {stdout}"
    );
}

/// `--show-sql --technique keyed_fold` on `daily_events` (`grain: partition`
/// — no cell ever admits `KeyedFold`) must report, per cell, that the
/// requested technique is not applicable — including the reason — rather
/// than silently omitting the cell (fail-loud discipline,
/// `docs/specs/ui_model_diagnostics.md` §Semantics "Admissibility
/// verdict": "a `NotApplicable` preview must never be rendered without its
/// reason").
#[test]
fn technique_flag_reports_not_applicable_per_cell() {
    let project_dir = timeseries_project_dir();
    let output = run_explain(&[
        "daily_events",
        "--show-sql",
        "--technique",
        "keyed_fold",
        "--project-dir",
        project_dir.to_str().expect("utf8 path"),
    ]);
    assert!(
        output.status.success(),
        "smelt explain daily_events --show-sql --technique keyed_fold failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        stdout.contains("technique preview: KeyedFold"),
        "expected a KeyedFold technique-preview header per cell: {stdout}"
    );
    assert!(
        stdout.contains("NotApplicable"),
        "daily_events's cells never admit KeyedFold — expected a NotApplicable verdict: {stdout}"
    );
    // The verdict line must carry a non-empty reason after the "—"
    // separator `format_admissibility` prints, never a bare
    // "NotApplicable" with nothing explaining why.
    let reason_line = stdout
        .lines()
        .find(|l| l.contains("NotApplicable"))
        .unwrap_or_else(|| panic!("expected a NotApplicable verdict line: {stdout}"));
    assert!(
        reason_line.contains("NotApplicable — ") && !reason_line.trim_end().ends_with("—"),
        "expected a non-empty reason after 'NotApplicable —': {reason_line}"
    );
}

/// `--show-sql --json` (no `--technique`) must include, per cell, the full
/// technique-preview array (all techniques the shared registry knows, not
/// just the admitted one) and a top-level `properties` object — both
/// previously absent from `ExplainMaintenanceJson`
/// (`docs/specs/ui_model_diagnostics.md` §Surface "CLI").
#[test]
fn json_includes_full_preview_array_and_properties() {
    let project_dir = timeseries_project_dir();
    let output = run_explain(&[
        "daily_events",
        "--show-sql",
        "--json",
        "--project-dir",
        project_dir.to_str().expect("utf8 path"),
    ]);
    assert!(
        output.status.success(),
        "smelt explain daily_events --show-sql --json failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);

    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("invalid JSON: {e}: {stdout}"));

    assert!(
        parsed.get("properties").is_some(),
        "expected a top-level 'properties' object: {stdout}"
    );
    let properties = &parsed["properties"];
    assert!(
        properties.get("columns").is_some() && properties.get("grain").is_some(),
        "expected the property set's own fields (columns, grain, ...): {properties}"
    );

    let cells = parsed
        .get("cells")
        .and_then(|c| c.as_array())
        .unwrap_or_else(|| panic!("expected a top-level 'cells' array: {stdout}"));
    assert!(!cells.is_empty(), "expected at least one cell: {stdout}");

    // The technique registry has 4 members (delete_insert, keyed_fold,
    // column_scoped_merge, in_place_update) — every cell's
    // `technique_previews` array must carry one entry per member, never
    // partial by omission, regardless of which one the plan admitted.
    for cell in cells {
        let previews = cell
            .get("technique_previews")
            .and_then(|p| p.as_array())
            .unwrap_or_else(|| panic!("expected a 'technique_previews' array on cell: {cell}"));
        assert_eq!(
            previews.len(),
            4,
            "expected one technique_previews entry per known technique: {cell}"
        );
        let has_admitted = previews.iter().any(|p| {
            p.get("admissibility")
                .and_then(|a| a.get("verdict"))
                .and_then(|v| v.as_str())
                == Some("admitted")
        });
        assert!(
            has_admitted,
            "expected exactly one Admitted entry in technique_previews: {cell}"
        );
    }
}
