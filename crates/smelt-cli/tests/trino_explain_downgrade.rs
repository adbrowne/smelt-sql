//! `docs/outcomes/20260913-trino-ledger/outcome.md` phase 4: `smelt explain`
//! must not abort on a `trino` target the way it did before this phase
//! (`smelt_backend::maintenance_dialect(SqlDialect::Trino)` returning `Err`
//! used to `?`-propagate straight out of the command). Availability
//! resolution and the maintenance-plan report are independent of the
//! maintenance-statement dialect; only the per-cell statement text
//! (`--show-sql`) is unavailable, and it must name the gap rather than
//! silently print nothing or fall back to another dialect's spelling.
//!
//! Offline only — `smelt explain` never opens a live connection, so a
//! placeholder `trino` target (no real coordinator) is enough.

use std::process::Command;

const SMELT_YML: &str = "name: trino_explain_downgrade_fixture\n\
    version: 1\n\
    paths:\n  - models\n\
    targets:\n  dev:\n    type: trino\n    host: trino.internal\n    port: 8080\n    \
    user: smelt\n    catalog: iceberg\n    schema: main\n\
    default_materialization: table\n";

const PAYMENTS_SOURCE: &str = "description: payments\n\
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

const REGION_MODEL_SQL: &str =
    "---\nmaterialization: table\nrefresh: incremental\ngrain: partition\n\
     timeseries:\n  partition_column: pay_date\n  event_time_column: pay_date\n  \
     granularity: day\n---\n\
     SELECT user_id, pay_date, amount FROM smelt.sources.payments\n";

/// A `refresh: incremental` / `grain: key` model (`Technique::KeyedFold`,
/// which needs the reconciliation ledger) on a `trino` target — Trino
/// realises no engine-resident state structure, so this cell must downgrade
/// to `PerGroupRecompute` (`docs/specs/state.md` §"The degradation
/// contract").
fn stage_trino_keyed_fold_project() -> tempfile::TempDir {
    let tmp = tempfile::TempDir::new().expect("create tempdir");
    std::fs::write(tmp.path().join("smelt.yml"), SMELT_YML).unwrap();
    std::fs::create_dir_all(tmp.path().join("models/sources")).unwrap();
    std::fs::write(
        tmp.path().join("models/sources/payments.yml"),
        PAYMENTS_SOURCE,
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("models/lifetime_spend.sql"),
        KEYED_FOLD_MODEL_SQL,
    )
    .unwrap();
    tmp
}

/// `smelt explain <model> --json` on a `trino` target exits 0 (never
/// aborts on the missing `MaintenanceDialect`) and its JSON carries a
/// `state_downgrade` whose `original` names the ideal (never-run)
/// technique — the ideal plan stays derived and visible even though it
/// cannot run.
#[test]
fn explain_on_a_trino_target_reports_instead_of_aborting() {
    let tmp = stage_trino_keyed_fold_project();

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
        "smelt explain must exit 0 on a trino target, not abort: stderr={}",
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
    let original = downgrade["original"]
        .as_str()
        .expect("state_downgrade.original must be a string");
    let executed = downgraded["technique"]
        .as_str()
        .expect("cell must carry the executed technique");
    assert_ne!(
        original, executed,
        "original must name the ideal technique, not the one actually executed: {stdout}"
    );
    assert_eq!(original, "KeyedFold");
    assert_eq!(executed, "PerGroupRecompute");
}

/// `smelt explain <model> --show-sql` on a `trino` target refuses by name —
/// naming Trino and the missing maintenance-statement support — rather
/// than printing nothing or silently falling back to another dialect's
/// spelling.
#[test]
fn explain_show_sql_on_trino_refuses_by_name() {
    let tmp = stage_trino_keyed_fold_project();

    let output = Command::new(env!("CARGO_BIN_EXE_smelt"))
        .arg("explain")
        .arg("lifetime_spend")
        .arg("--show-sql")
        .arg("--project-dir")
        .arg(tmp.path())
        .output()
        .expect("spawn smelt explain lifetime_spend --show-sql");

    assert!(
        output.status.success(),
        "smelt explain --show-sql must exit 0 on a trino target, not abort: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Trino"),
        "the show-sql leg must name Trino rather than staying silent: {stdout}"
    );
    assert!(
        stdout.contains("no statements") || stdout.contains("no maintenance-statement dialect"),
        "the show-sql leg must name the missing maintenance-statement support, not print a \
         substituted dialect's SQL: {stdout}"
    );
}

/// `smelt rebuild <model> --dry-run` on a `trino` target must not silently
/// `continue` past a model whose maintenance statements cannot be rendered
/// (`execute/project/dry_run.rs`'s old `let Ok(dialect) = ... else {
/// continue };`) — that made a skipped statement look identical to an
/// absent one (fail-loud discipline, `CLAUDE.md` §"Fail-loud discipline").
/// The compiled-SQL line still prints (`reporter.model_compiled`); a named
/// line about the missing maintenance-statement support must print too.
#[test]
fn dry_run_on_trino_names_the_gap() {
    let tmp = tempfile::TempDir::new().expect("create tempdir");
    std::fs::write(tmp.path().join("smelt.yml"), SMELT_YML).unwrap();
    std::fs::create_dir_all(tmp.path().join("models/sources")).unwrap();
    std::fs::write(
        tmp.path().join("models/sources/payments.yml"),
        PAYMENTS_SOURCE,
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("models/lifetime_spend.sql"),
        REGION_MODEL_SQL,
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_smelt"))
        .arg("rebuild")
        .arg("lifetime_spend")
        .arg("--start")
        .arg("2026-01-01")
        .arg("--end")
        .arg("2026-01-02")
        .arg("--dry-run")
        .arg("--project-dir")
        .arg(tmp.path())
        .output()
        .expect("spawn smelt rebuild lifetime_spend --dry-run");

    assert!(
        output.status.success(),
        "rebuild --dry-run must exit 0 on a trino target, not abort: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stdout.contains("Would run: lifetime_spend"),
        "the model's compiled-SQL line must still print: {stdout}"
    );
    assert!(
        stderr.contains("lifetime_spend") && stderr.contains("Trino"),
        "a named line about the missing maintenance-statement support must print, naming \
         the model and Trino, not a silent skip: stdout={stdout}\nstderr={stderr}"
    );
}
