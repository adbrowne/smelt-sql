use std::process::Command;

// --- Phase 6 (`docs/outcomes/20260904-state-residency/outcome.md`): the
// recorded `state_downgrade` becomes visible `smelt explain` surface. ---

const KEYED_FOLD_SMELT_YML_TEMPLATE: &str = "name: state_downgrade_fixture\n\
    version: 1\n\
    paths:\n  - models\n\
    targets:\n  dev:\n    type: {TYPE}\n    schema: main\n\
    default_materialization: view\n{STATE_BLOCK}";

const KEYED_FOLD_PAYMENTS_SOURCE: &str = "description: payments\n\
    mutation_profile: append_only\n\
    timeseries:\n  event_time_column: pay_date\n  partition_column: pay_date\n  granularity: day\n\
    columns:\n\
    - name: user_id\n  type: INTEGER\n\
    - name: pay_date\n  type: DATE\n\
    - name: amount\n  type: DOUBLE\n";

const KEYED_FOLD_MODEL_SQL: &str =
    "---\nmaterialization: table\nrefresh: incremental\ngrain: key\n---\n\
     SELECT user_id, SUM(amount) AS lifetime_spend\n\
     FROM smelt.sources.payments\nGROUP BY user_id\n";

/// Stage a fixture project with a keyed-fold model (`Technique::KeyedFold`,
/// which needs the reconciliation ledger) whose target dialect is `dialect`
/// and whose `state.warehouse_tables` is `state_block` (empty string for the
/// default `allowed`).
fn stage_keyed_fold_project(dialect: &str, state_block: &str) -> tempfile::TempDir {
    let tmp = tempfile::TempDir::new().expect("create tempdir");
    let smelt_yml = KEYED_FOLD_SMELT_YML_TEMPLATE
        .replace("{TYPE}", dialect)
        .replace("{STATE_BLOCK}", state_block);
    std::fs::write(tmp.path().join("smelt.yml"), smelt_yml).unwrap();
    std::fs::create_dir_all(tmp.path().join("models/sources")).unwrap();
    std::fs::write(
        tmp.path().join("models/sources/payments.yml"),
        KEYED_FOLD_PAYMENTS_SOURCE,
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("models/lifetime_spend.sql"),
        KEYED_FOLD_MODEL_SQL,
    )
    .unwrap();
    tmp
}

/// A keyed-fold model targeting a ledger-less dialect (Spark) prints a
/// `state downgrade:` row under the downgraded cell's `technique:` row,
/// naming the original technique and the missing structure.
#[test]
fn explain_text_prints_state_downgrade() {
    let tmp = stage_keyed_fold_project("spark", "");

    let output = Command::new(env!("CARGO_BIN_EXE_smelt"))
        .arg("explain")
        .arg("lifetime_spend")
        .arg("--project-dir")
        .arg(tmp.path())
        .output()
        .expect("spawn smelt explain lifetime_spend");

    assert!(
        output.status.success(),
        "smelt explain lifetime_spend failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("technique: PerGroupRecompute"),
        "expected the downgraded technique to be printed: {stdout}"
    );
    assert!(
        stdout.contains("state downgrade:")
            && stdout.contains("KeyedFold")
            && stdout.contains("reconciliation ledger"),
        "expected a state-downgrade row naming the original technique and missing structure: \
         {stdout}"
    );
}

/// The same model's `--json` output carries `cells[0].state_downgrade` with
/// `original`, `missing`, `reason`.
#[test]
fn explain_json_carries_state_downgrade() {
    let tmp = stage_keyed_fold_project("spark", "");

    let output = Command::new(env!("CARGO_BIN_EXE_smelt"))
        .arg("explain")
        .arg("lifetime_spend")
        .arg("--json")
        .arg("--project-dir")
        .arg(tmp.path())
        .output()
        .expect("spawn smelt explain lifetime_spend --json");

    assert!(
        output.status.success(),
        "smelt explain lifetime_spend --json failed: stderr={}",
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
    let downgrade = &downgraded["state_downgrade"];
    assert_eq!(downgrade["original"], "KeyedFold");
    assert!(downgrade["missing"]
        .as_str()
        .unwrap()
        .contains("reconciliation ledger"));
    assert!(downgrade["reason"].as_str().unwrap().contains("KeyedFold"));
}

/// A DuckDB target realises the reconciliation ledger, so no downgrade
/// occurs: no `state downgrade:` row in text, no `state_downgrade` key in
/// any JSON cell.
#[test]
fn explain_omits_state_downgrade_on_duckdb() {
    let tmp = stage_keyed_fold_project("duckdb", "");

    let text_output = Command::new(env!("CARGO_BIN_EXE_smelt"))
        .arg("explain")
        .arg("lifetime_spend")
        .arg("--project-dir")
        .arg(tmp.path())
        .output()
        .expect("spawn smelt explain lifetime_spend");
    assert!(text_output.status.success());
    let stdout = String::from_utf8_lossy(&text_output.stdout);
    assert!(
        !stdout.contains("state downgrade:"),
        "a DuckDB target must not print a downgrade row: {stdout}"
    );

    let json_output = Command::new(env!("CARGO_BIN_EXE_smelt"))
        .arg("explain")
        .arg("lifetime_spend")
        .arg("--json")
        .arg("--project-dir")
        .arg(tmp.path())
        .output()
        .expect("spawn smelt explain lifetime_spend --json");
    assert!(json_output.status.success());
    let json_stdout = String::from_utf8_lossy(&json_output.stdout);
    let json: serde_json::Value = serde_json::from_str(&json_stdout)
        .unwrap_or_else(|e| panic!("explain --json output must parse: {e}\n{json_stdout}"));
    let cells = json["cells"].as_array().expect("cells array");
    assert!(
        cells.iter().all(|c| c.get("state_downgrade").is_none()),
        "a DuckDB target must carry no state_downgrade key on any cell: {json_stdout}"
    );
}

/// `state.warehouse_tables: none` forces the downgrade even on DuckDB
/// (criterion 5's observable consequence — `state.md` §"Opting out of
/// warehouse bookkeeping").
#[test]
fn warehouse_tables_none_downgrades_on_duckdb() {
    let tmp = stage_keyed_fold_project("duckdb", "state:\n  warehouse_tables: none\n");

    let output = Command::new(env!("CARGO_BIN_EXE_smelt"))
        .arg("explain")
        .arg("lifetime_spend")
        .arg("--project-dir")
        .arg(tmp.path())
        .output()
        .expect("spawn smelt explain lifetime_spend");

    assert!(
        output.status.success(),
        "smelt explain lifetime_spend failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("state downgrade:"),
        "warehouse_tables: none must force a downgrade even on DuckDB: {stdout}"
    );
}
