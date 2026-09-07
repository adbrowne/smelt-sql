use std::path::Path;

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
