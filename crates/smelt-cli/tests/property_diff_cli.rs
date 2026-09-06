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

/// Stage a git repo at `examples/web_analytics`, committed on `main` — the
/// counterpart to [`stage_timeseries_repo`] for fixtures that need a source
/// with no declared timeseries clock (`raw.devices`).
fn stage_web_analytics_repo(tmp: &Path) {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples/web_analytics");
    copy_dir(&repo_root, tmp);
    git(tmp, &["init", "-q", "-b", "main"]);
    git(tmp, &["add", "-A"]);
    git_commit(tmp, "initial import of examples/web_analytics");
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
    assert_eq!(
        keys,
        vec!["baseline", "edited_files", "headline", "models", "summary"]
    );

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

/// Fix round 1, Q1 (critical): `profiles_for_workspace` must profile every
/// model, maintained or not — otherwise a `refresh: incremental` →
/// `refresh: full` edit makes the model vanish from the new profile map
/// entirely (never present-with-empty-cells), which routes through
/// `whole_model_changes` and gets graded all-`Neutral`, so losing
/// incremental maintenance reports ZERO downgrades and `--fail-on
/// downgrade` stays green — silently reintroducing the exact defect the
/// `maintenance_lost` dimension exists to catch. This is the headline case
/// end to end, against the real CLI.
#[test]
fn losing_incremental_maintenance_reports_a_maintenance_lost_downgrade() {
    let tmp = tempfile::tempdir().expect("tempdir");
    stage_timeseries_repo(tmp.path());

    let model_path = tmp.path().join("models/user_daily_spend.sql");
    let original = std::fs::read_to_string(&model_path).expect("read user_daily_spend.sql");
    assert!(
        original.contains("refresh: incremental"),
        "the fixture's `refresh:` line must match what this test replaces"
    );
    let edited = original.replacen("refresh: incremental", "refresh: full", 1);
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
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("parse json");
    let models = json["models"].as_array().expect("models array");
    let edited_model = models
        .iter()
        .find(|m| m["model"] == "user_daily_spend")
        .unwrap_or_else(|| panic!("user_daily_spend must be reported shifted: {json}"));
    assert_eq!(edited_model["cause"]["kind"], "edited");
    let changes = edited_model["changes"].as_array().expect("changes array");
    assert!(
        changes
            .iter()
            .any(|c| c["dimension"] == "maintenance_lost" && c["direction"] == "downgrade"),
        "user_daily_spend must show a maintenance_lost downgrade when refresh flips to full: \
         {changes:?}"
    );

    let output = smelt()
        .args([
            "explain",
            "--diff",
            "--fail-on",
            "downgrade",
            "--project-dir",
        ])
        .arg(tmp.path())
        .output()
        .expect("spawn smelt explain --diff --fail-on downgrade");
    assert_eq!(
        output.status.code(),
        Some(1),
        "--fail-on downgrade must exit 1 when maintenance_lost fired: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Fix round 1, Q2: a model whose body no longer derives a profile at all
/// (a `DELETE FROM …` statement, say — the reviewer's own repro) must be
/// reported `removed` WITH a `reason`, never a bare unexplained absence
/// (`docs/specs/property_diff.md` §Constraints item 6). Q1's fix (every
/// model gets a profile-or-failure verdict) is what makes this reachable:
/// before it, only a maintained model's derivation failure was ever
/// captured, and `user_daily_spend`'s baseline version — a plain
/// `refresh: incremental` SELECT — always had a maintenance plan, so this
/// path was live only in principle.
#[test]
fn a_body_that_no_longer_derives_a_profile_is_reported_removed_with_a_reason() {
    let tmp = tempfile::tempdir().expect("tempdir");
    stage_timeseries_repo(tmp.path());

    let model_path = tmp.path().join("models/user_daily_spend.sql");
    let original = std::fs::read_to_string(&model_path).expect("read user_daily_spend.sql");
    // Keep the frontmatter (so it is still the same declared model), but
    // replace the body with a non-SELECT statement `PropertySet::derive`
    // cannot turn into a property vector.
    let frontmatter_end = original.find("---\n").map(|i| i + 4).unwrap_or(0);
    let frontmatter_end = original[frontmatter_end..]
        .find("---\n")
        .map(|i| frontmatter_end + i + 4)
        .expect("fixture has a frontmatter block");
    let mut edited = original[..frontmatter_end].to_string();
    edited.push_str("DELETE FROM smelt.sources.raw.transactions WHERE amount < 0\n");
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
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("parse json");
    let models = json["models"].as_array().expect("models array");
    let m = models
        .iter()
        .find(|m| m["model"] == "user_daily_spend")
        .unwrap_or_else(|| panic!("user_daily_spend must be reported shifted: {json}"));
    // Whichever side the derivation failure lands on, it must carry a
    // reason — never a bare "removed"/"added" with none.
    assert!(
        m["cause"]["kind"] == "removed" || m["cause"]["kind"] == "added",
        "expected removed or added, got {:?}",
        m["cause"]
    );
    assert!(
        m["cause"]["reason"].is_string(),
        "a derivation failure must carry cause.reason, never a bare unexplained absence: {:?}",
        m["cause"]
    );
}

/// Fix round 1, Q4: `--fail-on` without `--diff` is a usage error, not a
/// silently ignored flag — the same class the other four exclusivity
/// checks already cover.
#[test]
fn fail_on_without_diff_is_a_usage_error() {
    let tmp = tempfile::tempdir().expect("tempdir");
    stage_timeseries_repo(tmp.path());

    let output = smelt()
        .args(["explain", "--fail-on", "any", "--project-dir"])
        .arg(tmp.path())
        .output()
        .expect("spawn smelt explain --fail-on any");
    assert_eq!(
        output.status.code(),
        Some(2),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

// --- `--markdown` (`docs/outcomes/20260905-property-diff/phases/06-plan.md`) ---

/// `--markdown` and `--json` together is a usage error — `conflicts_with =
/// "json"` on the clap declaration. Fails against a flag declared without
/// that attribute, which would silently print JSON and ignore `--markdown`.
#[test]
fn markdown_and_json_together_is_a_usage_error() {
    let tmp = tempfile::tempdir().expect("tempdir");
    stage_timeseries_repo(tmp.path());

    let output = smelt()
        .args(["explain", "--diff", "--markdown", "--json", "--project-dir"])
        .arg(tmp.path())
        .output()
        .expect("spawn smelt explain --diff --markdown --json");
    assert_eq!(
        output.status.code(),
        Some(2),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// `--markdown` without `--diff` is a usage error — `requires = "diff"`.
/// Fails against a missing `requires`, the exact hole Phase 5's Q4 found on
/// `--fail-on`.
#[test]
fn markdown_without_diff_is_a_usage_error() {
    let tmp = tempfile::tempdir().expect("tempdir");
    stage_timeseries_repo(tmp.path());

    let output = smelt()
        .args(["explain", "--markdown", "--project-dir"])
        .arg(tmp.path())
        .output()
        .expect("spawn smelt explain --markdown");
    assert_eq!(
        output.status.code(),
        Some(2),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Apply the join-induced `cell_technique` downgrade edit to
/// `user_daily_spend.sql`, used by several tests below (re-created from the
/// criterion-4 fixture already staged elsewhere in this file).
fn apply_join_downgrade_edit(tmp: &Path) {
    let model_path = tmp.join("models/user_daily_spend.sql");
    let original = std::fs::read_to_string(&model_path).expect("read user_daily_spend.sql");
    let edited = original.replace(
        "SELECT\n    user_id,\n    CAST(transaction_timestamp AS DATE) AS spend_date,\n    SUM(amount) AS total_amount\nFROM smelt.sources.raw.transactions\nGROUP BY 1, 2",
        "SELECT\n    t.user_id,\n    CAST(t.transaction_timestamp AS DATE) AS spend_date,\n    SUM(t.amount) AS total_amount\nFROM smelt.sources.raw.transactions t\nJOIN smelt.sources.raw.users u ON t.user_id = u.user_id\nGROUP BY 1, 2",
    );
    std::fs::write(&model_path, edited).expect("write edited user_daily_spend.sql");
}

/// `--markdown` reports the join downgrade in an open `<details>` block for
/// both the directly-edited model and its downstream dependent. Fails
/// against a renderer whose open-state or cause string is wrong, and
/// against a `print!` branch wired after the `--fail-on` early return
/// (there's no `--fail-on` here, so this alone only covers the render
/// path — test 10 covers the ordering hazard).
#[test]
fn markdown_reports_the_join_downgrade_in_an_open_details() {
    let tmp = tempfile::tempdir().expect("tempdir");
    stage_timeseries_repo(tmp.path());
    apply_join_downgrade_edit(tmp.path());

    let output = smelt()
        .args(["explain", "--diff", "--markdown", "--project-dir"])
        .arg(tmp.path())
        .output()
        .expect("spawn smelt explain --diff --markdown");
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("<!-- smelt-property-diff -->"),
        "expected the marker: {stdout}"
    );
    assert!(
        stdout.contains("<details open>\n<summary>user_daily_spend"),
        "expected an open details block naming user_daily_spend: {stdout}"
    );
    assert!(
        stdout.contains("<details open>\n<summary>user_spend_running_total"),
        "expected an open details block naming user_spend_running_total: {stdout}"
    );
}

/// `--markdown --fail-on downgrade` exits `1` on the join-downgrade edit
/// AND stdout still carries the full Markdown body. Fails against the
/// ordering hazard where the body is printed after the `--fail-on` early
/// return: the comment body would be empty exactly when it matters most —
/// a PR carrying a downgrade.
#[test]
fn markdown_body_is_printed_even_when_fail_on_exits_1() {
    let tmp = tempfile::tempdir().expect("tempdir");
    stage_timeseries_repo(tmp.path());
    apply_join_downgrade_edit(tmp.path());

    let output = smelt()
        .args([
            "explain",
            "--diff",
            "--markdown",
            "--fail-on",
            "downgrade",
            "--project-dir",
        ])
        .arg(tmp.path())
        .output()
        .expect("spawn smelt explain --diff --markdown --fail-on downgrade");
    assert_eq!(
        output.status.code(),
        Some(1),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("<!-- smelt-property-diff -->"),
        "the body must still be printed on a --fail-on exit: {stdout}"
    );
    assert!(
        stdout.contains("user_daily_spend"),
        "expected the full body, not an empty one: {stdout}"
    );
}

/// A formatting-only edit renders the cleared Markdown body: heading +
/// `no models shifted` + marker (pairs the unit test through the real
/// binary).
#[test]
fn a_formatting_only_edit_renders_the_cleared_markdown_body() {
    let tmp = tempfile::tempdir().expect("tempdir");
    stage_timeseries_repo(tmp.path());
    std::fs::write(
        tmp.path().join("README.md"),
        "# edited, but not a model, source, or smelt.yml file\n",
    )
    .expect("write README.md");

    let output = smelt()
        .args(["explain", "--diff", "--markdown", "--project-dir"])
        .arg(tmp.path())
        .output()
        .expect("spawn smelt explain --diff --markdown");
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("no models shifted"), "stdout={stdout}");
    assert!(
        stdout.trim_end().ends_with("<!-- smelt-property-diff -->"),
        "expected the marker as the last line: {stdout}"
    );
}

/// `docs/specs/property_diff.md` §Design "A new dependency is a cost, not an
/// upgrade": joining an unclocked dimension (`raw.devices` declares no
/// `timeseries:`, so a maintenance cell reading it has no partition to scan)
/// into an already-maintained model must grade `cell_added` a `downgrade`,
/// never an `upgrade` (`docs/specs/property_diff.md` §"Direction", `cell_added` row).
#[test]
fn new_unclocked_join_is_a_cell_added_downgrade() {
    let tmp = tempfile::tempdir().expect("tempdir");
    stage_web_analytics_repo(tmp.path());

    let model_path = tmp.path().join("models/gold/eventstream_with_identity.sql");
    let original =
        std::fs::read_to_string(&model_path).expect("read eventstream_with_identity.sql");
    let edited = original
        .replace(
            "FROM smelt.silver.events_deduped e\nJOIN smelt.silver.sessions s",
            "FROM smelt.silver.events_deduped e\nJOIN smelt.sources.raw.devices d ON e.device_id = d.device_id\nJOIN smelt.silver.sessions s",
        )
        .replace(
            "    s.session_id,\n    COALESCE(f.forward_only_amplitude_id,",
            "    s.session_id,\n    d.device_type,\n    COALESCE(f.forward_only_amplitude_id,",
        );
    assert_ne!(
        original, edited,
        "the fixture's SELECT/FROM text must match what this test replaces"
    );
    std::fs::write(&model_path, edited).expect("write edited eventstream_with_identity.sql");

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
        .find(|m| m["model"] == "gold.eventstream_with_identity")
        .unwrap_or_else(|| {
            panic!("gold.eventstream_with_identity must be reported shifted: {json}")
        });
    let changes = edited_model["changes"].as_array().expect("changes array");

    let cell_added = changes
        .iter()
        .find(|c| c["dimension"] == "cell_added")
        .unwrap_or_else(|| panic!("expected a cell_added change: {changes:?}"));
    assert_eq!(
        cell_added["direction"], "downgrade",
        "a non-partition-local new dependency must downgrade: {cell_added}"
    );
    assert_eq!(
        cell_added["new"]["partition_local"], false,
        "the new cell must not be partition-local: {cell_added}"
    );

    assert!(
        !changes.iter().any(|c| c["direction"] == "upgrade"),
        "a new dependency must never report an upgrade: {changes:?}"
    );
}

/// `docs/specs/property_diff.md` §"Direction" grain row: widening the grain
/// (adding `user_name` to `daily_revenue`'s key via a dimension join) must
/// grade `grain` a `downgrade` (§"Direction": a widened grain is a weaker uniqueness claim).
#[test]
fn key_widening_join_is_a_grain_downgrade() {
    let tmp = tempfile::tempdir().expect("tempdir");
    stage_timeseries_repo(tmp.path());

    let model_path = tmp.path().join("models/daily_revenue.sql");
    let original = std::fs::read_to_string(&model_path).expect("read daily_revenue.sql");
    let edited = original
        .replace(
            "SELECT\n    CAST(transaction_timestamp AS DATE) as revenue_date,\n    user_id,\n    COUNT(*) as transaction_count,\n    SUM(amount) as total_revenue,\n    AVG(amount) as avg_transaction_amount,\n    MIN(transaction_timestamp) as first_transaction,\n    MAX(transaction_timestamp) as last_transaction,\nFROM smelt.sources.raw.transactions\nWHERE transaction_timestamp IS NOT NULL\nGROUP BY 1, 2\nORDER BY 1, 2",
            "SELECT\n    CAST(t.transaction_timestamp AS DATE) as revenue_date,\n    t.user_id,\n    u.user_name,\n    COUNT(*) as transaction_count,\n    SUM(t.amount) as total_revenue,\n    AVG(t.amount) as avg_transaction_amount,\n    MIN(t.transaction_timestamp) as first_transaction,\n    MAX(t.transaction_timestamp) as last_transaction,\nFROM smelt.sources.raw.transactions t\nJOIN smelt.sources.raw.users u ON t.user_id = u.user_id\nWHERE t.transaction_timestamp IS NOT NULL\nGROUP BY 1, 2, 3\nORDER BY 1, 2, 3",
        );
    assert_ne!(
        original, edited,
        "the fixture's SELECT text must match what this test replaces"
    );
    std::fs::write(&model_path, edited).expect("write edited daily_revenue.sql");

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
        .find(|m| m["model"] == "daily_revenue")
        .unwrap_or_else(|| panic!("daily_revenue must be reported shifted: {json}"));
    let changes = edited_model["changes"].as_array().expect("changes array");

    let grain_change = changes
        .iter()
        .find(|c| c["dimension"] == "grain")
        .unwrap_or_else(|| panic!("expected a grain change: {changes:?}"));
    assert_eq!(
        grain_change["direction"], "downgrade",
        "a widened grain must downgrade: {grain_change}"
    );
}
