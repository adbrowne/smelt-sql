//! `smelt explain --diff` (`docs/specs/property_diff.md`): end-to-end CLI
//! coverage over real temp git repos, spawning the real `smelt` binary.
//!
//! Git helpers below are re-created from `crates/smelt-core/tests/baseline.rs`
//! (`git`, `git_commit`) and `crates/smelt-cli/tests/exit_codes.rs`
//! (`copy_dir`) — the same shapes, not new inventions; both are
//! test-binary-local modules that cannot be imported across crates.

use std::path::Path;
use std::process::Command;

fn git(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git must be available to run these tests");
    assert!(
        output.status.success(),
        "git {:?} failed in {:?}: {}",
        args,
        dir,
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_commit(dir: &Path, message: &str) {
    let output = Command::new("git")
        .args([
            "-c",
            "user.email=t@example.invalid",
            "-c",
            "user.name=t",
            "commit",
            "-m",
            message,
        ])
        .current_dir(dir)
        .output()
        .expect("git must be available to run these tests");
    assert!(
        output.status.success(),
        "git commit failed in {:?}: {}",
        dir,
        String::from_utf8_lossy(&output.stderr)
    );
}

fn copy_dir(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap().flatten() {
        let path = entry.path();
        let name = entry.file_name();
        if name == "target" || name == ".smelt" || name == ".git" {
            continue;
        }
        let dest = dst.join(&name);
        if path.is_dir() {
            copy_dir(&path, &dest);
        } else {
            std::fs::copy(&path, &dest).unwrap();
        }
    }
}

/// Stage a git repo at `examples/timeseries`, committed on `main`. Returns
/// the repo's root (== the project dir, since the example has no nested
/// project layout).
fn stage_timeseries_repo(tmp: &Path) {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples/timeseries");
    copy_dir(&repo_root, tmp);
    git(tmp, &["init", "-q", "-b", "main"]);
    git(tmp, &["add", "-A"]);
    git_commit(tmp, "initial import of examples/timeseries");
}

fn smelt() -> Command {
    Command::new(env!("CARGO_BIN_EXE_smelt"))
}

/// `--diff` conflicts with the positional model, `--show-sql`, `--period`,
/// and `--technique` — clap-enforced, exit `2`
/// (`docs/specs/property_diff.md` §Surface).
#[test]
fn diff_with_a_model_argument_is_a_usage_error() {
    let tmp = tempfile::tempdir().expect("tempdir");
    stage_timeseries_repo(tmp.path());

    // D4: `--diff` greedily consumes the next token as its ref, so the
    // positional model name must be written BEFORE the flag for this to be
    // the exclusivity case (not "ref named after the model").
    let output = smelt()
        .args(["explain", "user_daily_spend", "--diff", "--project-dir"])
        .arg(tmp.path())
        .output()
        .expect("spawn smelt explain <model> --diff");
    assert_eq!(
        output.status.code(),
        Some(2),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let output = smelt()
        .args(["explain", "--diff", "--show-sql", "--project-dir"])
        .arg(tmp.path())
        .output()
        .expect("spawn smelt explain --diff --show-sql");
    assert_eq!(output.status.code(), Some(2));

    let output = smelt()
        .args([
            "explain",
            "--diff",
            "--period",
            "2024-01-01..2024-01-02",
            "--project-dir",
        ])
        .arg(tmp.path())
        .output()
        .expect("spawn smelt explain --diff --period");
    assert_eq!(output.status.code(), Some(2));

    let output = smelt()
        .args([
            "explain",
            "--diff",
            "--show-sql",
            "--technique",
            "keyed_fold",
            "--project-dir",
        ])
        .arg(tmp.path())
        .output()
        .expect("spawn smelt explain --diff --show-sql --technique");
    assert_eq!(output.status.code(), Some(2));
}

/// A project outside any git work tree cannot resolve a baseline — usage
/// error, exit `2`.
#[test]
fn diff_outside_a_git_work_tree_exits_2() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples/timeseries");
    copy_dir(&repo_root, tmp.path());
    // Deliberately no `git init`.

    let output = smelt()
        .args(["explain", "--diff", "--project-dir"])
        .arg(tmp.path())
        .output()
        .expect("spawn smelt explain --diff");
    assert_eq!(
        output.status.code(),
        Some(2),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// An explicit ref that does not resolve is a usage error, exit `2`.
#[test]
fn diff_with_an_unknown_ref_exits_2() {
    let tmp = tempfile::tempdir().expect("tempdir");
    stage_timeseries_repo(tmp.path());

    let output = smelt()
        .args(["explain", "--diff", "nonexistent-ref-xyz", "--project-dir"])
        .arg(tmp.path())
        .output()
        .expect("spawn smelt explain --diff <bad ref>");
    assert_eq!(
        output.status.code(),
        Some(2),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Criterion 4's fixture test — the entire reason this feature exists: a
/// silent downstream downgrade must be caught, named, and attributed.
///
/// **Deviation from the plan's literal "`SUM` → `MAX`" edit, recorded per
/// ruling R3.** A plain `SUM(amount)` → `MAX(amount)` edit on
/// `user_daily_spend` was verified BY HAND (see `phases/05-summary.md`) to
/// produce no shift at all in `examples/timeseries`: the only
/// combiner-sensitive cell is a `NewData` fold over the append-only
/// `raw.transactions` source, which never needs a correction/`
/// UpstreamMutation` cell, so `KeyedFold`'s forward-fold admission is
/// insensitive to whether the combiner is invertible (`SUM`, `MAX`, `AVG`,
/// even `SUM(DISTINCT ...)` all stayed admitted; only a truly holistic
/// combiner like `MEDIAN` refused the cell outright, with zero downstream
/// propagation either way). The edit that DOES reproduce the silent
/// downstream downgrade — verified by hand against this exact fixture — is
/// adding a join to an unclocked dimension (`raw.users`) inside
/// `user_daily_spend`, which breaks its row identity/partition-locality
/// proof and downgrades its `NewData` cell's technique; that shift
/// propagates to `user_spend_running_total` (its direct downstream
/// consumer) as a lost cell, attributed `downstream of user_daily_spend`.
#[test]
fn a_join_induced_downgrade_propagates_to_the_named_downstream_model() {
    let tmp = tempfile::tempdir().expect("tempdir");
    stage_timeseries_repo(tmp.path());

    let model_path = tmp.path().join("models/user_daily_spend.sql");
    let original = std::fs::read_to_string(&model_path).expect("read user_daily_spend.sql");
    let edited = original.replace(
        "SELECT\n    user_id,\n    CAST(transaction_timestamp AS DATE) AS spend_date,\n    SUM(amount) AS total_amount\nFROM smelt.sources.raw.transactions\nGROUP BY 1, 2",
        "SELECT\n    t.user_id,\n    CAST(t.transaction_timestamp AS DATE) AS spend_date,\n    SUM(t.amount) AS total_amount\nFROM smelt.sources.raw.transactions t\nJOIN smelt.sources.raw.users u ON t.user_id = u.user_id\nGROUP BY 1, 2",
    );
    assert_ne!(
        original, edited,
        "the fixture's SELECT text must match what this test replaces"
    );
    std::fs::write(&model_path, edited).expect("write edited user_daily_spend.sql");

    let output = smelt()
        .args(["explain", "--diff", "--json", "--project-dir"])
        .arg(tmp.path())
        .output()
        .expect("spawn smelt explain --diff --json");
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let json: serde_json::Value = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|e| panic!("--json did not print JSON: {e}\n{output:?}"));

    let models = json["models"].as_array().expect("models must be an array");
    let edited_model = models
        .iter()
        .find(|m| m["model"] == "user_daily_spend")
        .unwrap_or_else(|| panic!("user_daily_spend must be reported shifted: {json}"));
    assert_eq!(edited_model["cause"]["kind"], "edited");
    let changes = edited_model["changes"].as_array().expect("changes array");
    assert!(
        changes
            .iter()
            .any(|c| c["dimension"] == "cell_technique" && c["direction"] == "downgrade"),
        "user_daily_spend must show a cell_technique downgrade: {changes:?}"
    );

    let downstream_model = models
        .iter()
        .find(|m| m["model"] == "user_spend_running_total")
        .unwrap_or_else(|| panic!("user_spend_running_total must be reported shifted: {json}"));
    assert_eq!(downstream_model["cause"]["kind"], "downstream");
    assert_eq!(
        downstream_model["cause"]["of"],
        serde_json::json!(["user_daily_spend"])
    );
}

/// A formatting-only edit (no SQL/frontmatter/config change) yields no
/// shifted models — the text form's single-line output.
#[test]
fn a_formatting_only_edit_yields_no_models_shifted() {
    let tmp = tempfile::tempdir().expect("tempdir");
    stage_timeseries_repo(tmp.path());

    // `edited_set`'s predicate compares FRONTMATTER-STRIPPED SQL text, so a
    // change entirely inside the frontmatter block that doesn't touch
    // parsed metadata would still count as edited — use a change to the SQL
    // body's whitespace/comment instead, which is invisible to the
    // predicate only if it does not change the frontmatter-stripped text.
    // The cheapest genuinely-invisible edit is a change to a file the
    // predicate does not look at all: `README.md`.
    std::fs::write(
        tmp.path().join("README.md"),
        "# edited, but not a model, source, or smelt.yml file\n",
    )
    .expect("write README.md");

    let output = smelt()
        .args(["explain", "--diff", "--project-dir"])
        .arg(tmp.path())
        .output()
        .expect("spawn smelt explain --diff");
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.trim_end().ends_with("no models shifted"),
        "expected the single 'no models shifted' line: {stdout}"
    );
}

/// `--fail-on downgrade` and `--fail-on any` both exit `1` when the
/// join-induced downgrade fixture is diffed; a genuinely unshifted repo
/// still exits `0` under `--fail-on any`.
#[test]
fn fail_on_downgrade_exits_1_and_fail_on_any_exits_1() {
    let tmp = tempfile::tempdir().expect("tempdir");
    stage_timeseries_repo(tmp.path());
    let model_path = tmp.path().join("models/user_daily_spend.sql");
    let original = std::fs::read_to_string(&model_path).expect("read user_daily_spend.sql");
    let edited = original.replace(
        "SELECT\n    user_id,\n    CAST(transaction_timestamp AS DATE) AS spend_date,\n    SUM(amount) AS total_amount\nFROM smelt.sources.raw.transactions\nGROUP BY 1, 2",
        "SELECT\n    t.user_id,\n    CAST(t.transaction_timestamp AS DATE) AS spend_date,\n    SUM(t.amount) AS total_amount\nFROM smelt.sources.raw.transactions t\nJOIN smelt.sources.raw.users u ON t.user_id = u.user_id\nGROUP BY 1, 2",
    );
    std::fs::write(&model_path, edited).expect("write edited user_daily_spend.sql");

    for fail_on in ["downgrade", "any"] {
        let output = smelt()
            .args(["explain", "--diff", "--fail-on", fail_on, "--project-dir"])
            .arg(tmp.path())
            .output()
            .unwrap_or_else(|e| panic!("spawn smelt explain --diff --fail-on {fail_on}: {e}"));
        assert_eq!(
            output.status.code(),
            Some(1),
            "--fail-on {fail_on}: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    // An unshifted repo (formatting-only edit) exits 0 even under
    // --fail-on any.
    let tmp2 = tempfile::tempdir().expect("tempdir");
    stage_timeseries_repo(tmp2.path());
    std::fs::write(tmp2.path().join("README.md"), "no model change\n").unwrap();
    let output = smelt()
        .args(["explain", "--diff", "--fail-on", "any", "--project-dir"])
        .arg(tmp2.path())
        .output()
        .expect("spawn smelt explain --diff --fail-on any (unshifted)");
    assert_eq!(output.status.code(), Some(0));
}

/// `--select` narrows the REPORTED set only; the narrowed entry's
/// attribution is unaffected, and the summary counts follow the reported
/// set (Δ2).
#[test]
fn select_narrows_the_reported_set_but_not_attribution() {
    let tmp = tempfile::tempdir().expect("tempdir");
    stage_timeseries_repo(tmp.path());
    let model_path = tmp.path().join("models/user_daily_spend.sql");
    let original = std::fs::read_to_string(&model_path).expect("read user_daily_spend.sql");
    let edited = original.replace(
        "SELECT\n    user_id,\n    CAST(transaction_timestamp AS DATE) AS spend_date,\n    SUM(amount) AS total_amount\nFROM smelt.sources.raw.transactions\nGROUP BY 1, 2",
        "SELECT\n    t.user_id,\n    CAST(t.transaction_timestamp AS DATE) AS spend_date,\n    SUM(t.amount) AS total_amount\nFROM smelt.sources.raw.transactions t\nJOIN smelt.sources.raw.users u ON t.user_id = u.user_id\nGROUP BY 1, 2",
    );
    std::fs::write(&model_path, edited).expect("write edited user_daily_spend.sql");

    let output = smelt()
        .args([
            "explain",
            "--diff",
            "--select",
            "user_spend_running_total",
            "--json",
            "--project-dir",
        ])
        .arg(tmp.path())
        .output()
        .expect("spawn smelt explain --diff --select ... --json");
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("parse json");
    let models = json["models"].as_array().expect("models array");
    assert_eq!(
        models.len(),
        1,
        "expected exactly the selected model: {json}"
    );
    assert_eq!(models[0]["model"], "user_spend_running_total");
    assert_eq!(models[0]["cause"]["kind"], "downstream");
    assert_eq!(
        models[0]["cause"]["of"],
        serde_json::json!(["user_daily_spend"]),
        "attribution must be unaffected by --select"
    );
    assert_eq!(json["summary"]["shifted_models"], 1);
}

/// The JSON output's top-level shape matches the spec schema.
#[test]
fn diff_json_top_level_matches_the_schema() {
    let tmp = tempfile::tempdir().expect("tempdir");
    stage_timeseries_repo(tmp.path());
    let model_path = tmp.path().join("models/user_daily_spend.sql");
    let original = std::fs::read_to_string(&model_path).expect("read user_daily_spend.sql");
    let edited = original.replace(
        "SELECT\n    user_id,\n    CAST(transaction_timestamp AS DATE) AS spend_date,\n    SUM(amount) AS total_amount\nFROM smelt.sources.raw.transactions\nGROUP BY 1, 2",
        "SELECT\n    t.user_id,\n    CAST(t.transaction_timestamp AS DATE) AS spend_date,\n    SUM(t.amount) AS total_amount\nFROM smelt.sources.raw.transactions t\nJOIN smelt.sources.raw.users u ON t.user_id = u.user_id\nGROUP BY 1, 2",
    );
    std::fs::write(&model_path, edited).expect("write edited user_daily_spend.sql");

    let output = smelt()
        .args(["explain", "--diff", "--json", "--project-dir"])
        .arg(tmp.path())
        .output()
        .expect("spawn smelt explain --diff --json");
    assert_eq!(output.status.code(), Some(0));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("parse json");

    let obj = json.as_object().expect("top-level object");
    let mut keys: Vec<&str> = obj.keys().map(|s| s.as_str()).collect();
    keys.sort();
    assert_eq!(keys, vec!["baseline", "edited_files", "models", "summary"]);

    let baseline = json["baseline"].as_object().expect("baseline object");
    let mut bkeys: Vec<&str> = baseline.keys().map(|s| s.as_str()).collect();
    bkeys.sort();
    assert_eq!(bkeys, vec!["commit", "ref", "resolved_as"]);

    let edited_files = json["edited_files"].as_array().expect("edited_files array");
    assert!(
        edited_files
            .iter()
            .any(|f| f == "models/user_daily_spend.sql"),
        "edited_files must contain the edited model's path: {edited_files:?}"
    );

    let summary = json["summary"].as_object().expect("summary object");
    for key in ["downgrades", "upgrades", "neutral", "shifted_models"] {
        assert!(
            summary.get(key).is_some(),
            "summary must carry the `{key}` field"
        );
    }
    assert_eq!(
        summary["shifted_models"].as_u64(),
        Some(json["models"].as_array().unwrap().len() as u64),
        "summary.shifted_models must match models.len()"
    );
}
