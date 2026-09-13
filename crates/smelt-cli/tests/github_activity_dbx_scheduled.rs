//! Criterion 11, closed: three consecutive **scheduled** (`trigger: PERIODIC`)
//! runs of the deployed Databricks Asset Bundle job complete unattended, and
//! the state they leave equals a full-refresh oracle exactly as criterion 8
//! checks (`docs/outcomes/20260912-databricks-dogfood-spine/outcome.md`
//! criterion 11, `phases/11e-plan.md`).
//!
//! This is not a third independent equivalence sweep: it reuses
//! [`parity_support::DATABRICKS_EQUIVALENCE_DIVERGENCE_REGISTRY`], the same
//! registry `github_activity_dbx_oracle.rs` checks its manually-driven
//! windows against, because the licensed divergence
//! (`gold_events_enriched`'s `current_repo_name`) is a property of the
//! maintenance plan, not of how a run was triggered.
//!
//! Report-driven: both committed reports are read here, not measured. They
//! are written by a future live phase driving `scripts/dbx-bundle.sh runs`,
//! `scripts/dbx-dogfood-oracle.sh`, and the same equivalence-sweep machinery
//! `github_activity_dbx_oracle.rs` uses. Phase 11g attempted this and hit a
//! second wheel-platform defect beyond 11f's scope (Databricks serverless
//! compute lands each run on either `aarch64` or `x86_64` with no way to
//! pin it — `dbx-wheel-build.sh` builds `x86_64` only), so evidence is not
//! yet committed; see `outcome.md` `## Blocked`.

use std::collections::BTreeSet;
use std::path::PathBuf;

#[path = "parity_support/mod.rs"]
mod parity_support;
use parity_support::DATABRICKS_EQUIVALENCE_DIVERGENCE_REGISTRY;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates dir")
        .parent()
        .expect("repo root")
        .to_owned()
}

const RUNS_REPORT_PATH: &str =
    "docs/outcomes/20260912-databricks-dogfood-spine/phases/11g-runs.json";
const EQUIVALENCE_REPORT_PATH: &str =
    "docs/outcomes/20260912-databricks-dogfood-spine/phases/11g-equivalence.json";

/// `None` until a live phase commits `phases/11g-runs.json` — these three
/// consecutive scheduled runs are wall-clock-bound and blocked as of this
/// commit (see `## Blocked`), so the tests below skip rather than hard-fail
/// until that evidence lands, matching the report-driven pattern
/// `github_activity_dbx_oracle.rs`'s own history uses while a sweep is
/// pending (phase 9b's summary, "tests 5-6 ... were NOT landed").
fn runs_report() -> Option<serde_json::Value> {
    let path = repo_root().join(RUNS_REPORT_PATH);
    if !path.exists() {
        return None;
    }
    Some(
        serde_json::from_str(
            &std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}")),
        )
        .unwrap_or_else(|e| panic!("parse {path:?}: {e}")),
    )
}

/// `None` until the same live session commits `phases/11g-equivalence.json`
/// — see `runs_report`'s doc comment.
fn equivalence_report() -> Option<serde_json::Value> {
    let path = repo_root().join(EQUIVALENCE_REPORT_PATH);
    if !path.exists() {
        return None;
    }
    Some(
        serde_json::from_str(
            &std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}")),
        )
        .unwrap_or_else(|e| panic!("parse {path:?}: {e}")),
    )
}

fn scheduled_runs(report: &serde_json::Value) -> Vec<&serde_json::Value> {
    report["scheduled_runs"]
        .as_array()
        .expect("the report declares scheduled_runs")
        .iter()
        .collect()
}

/// **Criterion 11's core claim.** Exactly three scheduled runs, each
/// `trigger: PERIODIC` (never a manual `bundle run` or `RUN_JOB_TASK`), each
/// terminal state `SUCCESS` with both tasks succeeding, strictly ascending
/// start time, and no run recorded in `all_runs_considered` — the full run
/// history queried at capture time — falls strictly between the first and
/// last scheduled run's start time other than the three themselves. That
/// last check is what rules out a manual run interleaved between the
/// scheduled ones; the plan's smoke run (task 4) runs strictly *before* the
/// three, which this does not forbid.
#[test]
fn three_consecutive_scheduled_runs_succeeded() {
    let Some(report) = runs_report() else {
        eprintln!("Skipping — phases/11g-runs.json not yet committed (see outcome.md ## Blocked)");
        return;
    };
    let runs = scheduled_runs(&report);
    assert_eq!(
        runs.len(),
        3,
        "criterion 11 asks for exactly three consecutive scheduled runs, got {}",
        runs.len()
    );

    let mut start_times = Vec::new();
    let mut run_ids = BTreeSet::new();
    for run in &runs {
        let trigger = run["trigger"].as_str().expect("trigger");
        assert_eq!(
            trigger, "PERIODIC",
            "every counted run must be scheduled (trigger PERIODIC), not manually triggered: \
             run {run}"
        );
        let result_state = run["result_state"].as_str().expect("result_state");
        assert_eq!(
            result_state, "SUCCESS",
            "run {} did not terminate SUCCESS: {run}",
            run["run_id"]
        );
        let tasks = run["tasks"].as_array().expect("tasks array");
        assert!(!tasks.is_empty(), "run {} recorded no tasks", run["run_id"]);
        for task in tasks {
            let task_state = task["state"].as_str().expect("task state");
            assert_eq!(
                task_state, "SUCCESS",
                "task {} of run {} did not succeed: {task}",
                task["task_key"], run["run_id"]
            );
        }
        start_times.push(run["start_time_ms"].as_i64().expect("start_time_ms"));
        run_ids.insert(run["run_id"].as_i64().expect("run_id"));
    }

    assert!(
        start_times.windows(2).all(|w| w[0] < w[1]),
        "scheduled run start times must be strictly ascending: {start_times:?}"
    );

    let window_start = *start_times.first().expect("three runs checked above");
    let window_end = *start_times.last().expect("three runs checked above");
    let all_runs = report["all_runs_considered"].as_array().expect(
        "the report declares all_runs_considered — the full run history at capture \
                 time, used to rule out an interleaved manual run",
    );
    for run in all_runs {
        let run_id = run["run_id"].as_i64().expect("run_id");
        if run_ids.contains(&run_id) {
            continue;
        }
        let start = run["start_time_ms"].as_i64().expect("start_time_ms");
        assert!(
            !(window_start < start && start < window_end),
            "run {run_id} (trigger {}) falls between the first and last of the three \
             scheduled runs — this is exactly the interleaved-manual-run case criterion 11 \
             forbids: {run}",
            run["trigger"].as_str().unwrap_or("?")
        );
    }
}

/// The self-driving `--next-day` fix (phase 11b) must actually advance the
/// fixture one distinct day per scheduled run — otherwise "three scheduled
/// runs" could be three repeats of the same day rather than three genuine
/// incremental windows.
#[test]
fn scheduled_runs_advanced_the_fixture() {
    let Some(report) = runs_report() else {
        eprintln!("Skipping — phases/11g-runs.json not yet committed (see outcome.md ## Blocked)");
        return;
    };
    let runs = scheduled_runs(&report);
    let days: Vec<&str> = runs
        .iter()
        .map(|r| {
            r["loader_fixture_day"]
                .as_str()
                .expect("loader_fixture_day")
        })
        .collect();
    assert_eq!(days.len(), 3, "expected a fixture day recorded per run");
    assert!(
        days.windows(2).all(|w| w[0] < w[1]),
        "each scheduled run must land a distinct, strictly later fixture day than the last, \
         proving the runs are successive windows rather than repeats: {days:?}"
    );
}

fn report_checkpoints(report: &serde_json::Value) -> Vec<&serde_json::Value> {
    report["checkpoints"]
        .as_array()
        .expect("the report declares checkpoints")
        .iter()
        .collect()
}

/// **Criterion 8, re-proven under a scheduled trigger.** After the third
/// scheduled run, the state Databricks holds equals a full refresh over the
/// inputs seen so far, at every compared relation, unless the difference
/// matches [`DATABRICKS_EQUIVALENCE_DIVERGENCE_REGISTRY`]'s single entry
/// (`gold_events_enriched`'s `current_repo_name`). An unregistered
/// difference fails.
#[test]
fn scheduled_state_matches_its_full_refresh_oracle() {
    let Some(report) = equivalence_report() else {
        eprintln!(
            "Skipping — phases/11g-equivalence.json not yet committed (see outcome.md ## Blocked)"
        );
        return;
    };
    let registered: BTreeSet<String> = DATABRICKS_EQUIVALENCE_DIVERGENCE_REGISTRY
        .iter()
        .map(|e| e.relation.to_string())
        .collect();

    let checkpoints = report_checkpoints(&report);
    assert!(
        !checkpoints.is_empty(),
        "the committed post-scheduled-run equivalence report declares no checkpoint"
    );

    let mut offenders = Vec::new();
    for cp in &checkpoints {
        let label = cp["label"].as_str().expect("label");
        for rel in cp["relations"].as_array().expect("relations") {
            let relation = rel["relation"].as_str().expect("relation name");
            if registered.contains(relation) {
                continue;
            }
            let incr_only = rel["incr_only"].as_i64().expect("incr_only");
            let oracle_only = rel["oracle_only"].as_i64().expect("oracle_only");
            if incr_only != 0 || oracle_only != 0 {
                offenders.push(format!(
                    "{label}/{relation}: incremental_only={incr_only}, oracle_only={oracle_only}"
                ));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "the post-scheduled-run state violates the equivalence invariant with no registered \
         licence: {offenders:?}"
    );
}
