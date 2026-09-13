//! Standing gate over `docs/handoffs/2026-09-13-databricks-findings.md`
//! (`docs/outcomes/20260912-databricks-dogfood-spine/outcome.md` criterion 9):
//! every relation the live dual-target parity sweep or the live equivalence-
//! oracle sweep measured as diverging — and every model either sweep excluded
//! — must be named in the handoff. A future sweep that surfaces a new,
//! unregistered divergence and forgets to write it up fails this test rather
//! than silently shipping an incomplete evidence bank.

use std::fs;
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

const HANDOFF_PATH: &str = "docs/handoffs/2026-09-13-databricks-findings.md";
const PARITY_REPORT_PATH: &str =
    "docs/outcomes/20260912-databricks-dogfood-spine/phases/08-parity.json";
const EQUIVALENCE_REPORT_PATH: &str =
    "docs/outcomes/20260912-databricks-dogfood-spine/phases/09b-equivalence.json";

fn handoff_text() -> String {
    let path = repo_root().join(HANDOFF_PATH);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"))
}

fn read_json(relative: &str) -> serde_json::Value {
    let path = repo_root().join(relative);
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {path:?}: {e}"))
}

/// Every relation `08-parity.json` measured with a nonzero `duck_only`/
/// `dbx_only` at any checkpoint, and every relation `09b-equivalence.json`
/// measured with a nonzero `incr_only`/`oracle_only` at any checkpoint, must
/// be named in the handoff — a divergence recorded in a report but not
/// written up is a silent gap in the evidence bank.
#[test]
fn the_handoff_names_every_measured_divergence() {
    let handoff = handoff_text();

    let parity = read_json(PARITY_REPORT_PATH);
    let equivalence = read_json(EQUIVALENCE_REPORT_PATH);

    let mut divergent: Vec<String> = Vec::new();
    for report in [&parity, &equivalence] {
        let checkpoints = report
            .get("checkpoints")
            .and_then(|v| v.as_array())
            .unwrap_or_else(|| panic!("report has no `checkpoints` array: {report}"));
        for cp in checkpoints {
            let relations = cp
                .get("relations")
                .and_then(|v| v.as_array())
                .unwrap_or_else(|| panic!("checkpoint has no `relations` array: {cp}"));
            for rel in relations {
                let name = rel
                    .get("relation")
                    .and_then(|v| v.as_str())
                    .unwrap_or_else(|| panic!("relation entry has no `relation` string: {rel}"));
                let nonzero = ["duck_only", "dbx_only", "incr_only", "oracle_only"]
                    .iter()
                    .any(|field| rel.get(*field).and_then(|v| v.as_u64()).unwrap_or(0) != 0);
                if nonzero {
                    divergent.push(name.to_string());
                }
            }
        }
    }
    divergent.sort();
    divergent.dedup();

    assert!(
        !divergent.is_empty(),
        "sanity check: expected at least one measured divergence (gold_events_enriched) in \
         the committed reports — the fixtures may have changed shape"
    );

    let missing: Vec<&String> = divergent
        .iter()
        .filter(|name| !handoff.contains(name.as_str()))
        .collect();

    assert!(
        missing.is_empty(),
        "docs/handoffs/2026-09-13-databricks-findings.md does not name every relation the \
         committed reports measured as diverging: {missing:?}"
    );
}

/// Every entry of either report's `excluded_models` must be named in the
/// handoff — a model silently dropped from a sweep is a recorded finding,
/// never an unmentioned gap.
#[test]
fn the_handoff_names_every_excluded_model() {
    let handoff = handoff_text();

    let parity = read_json(PARITY_REPORT_PATH);
    let equivalence = read_json(EQUIVALENCE_REPORT_PATH);

    let mut excluded: Vec<String> = Vec::new();
    for report in [&parity, &equivalence] {
        let models = report
            .get("excluded_models")
            .and_then(|v| v.as_array())
            .unwrap_or_else(|| panic!("report has no `excluded_models` array: {report}"));
        for m in models {
            let name = m
                .as_str()
                .unwrap_or_else(|| panic!("excluded_models entry is not a string: {m}"));
            excluded.push(name.to_string());
        }
    }
    excluded.sort();
    excluded.dedup();

    let missing: Vec<&String> = excluded
        .iter()
        .filter(|name| !handoff.contains(name.as_str()))
        .collect();

    assert!(
        missing.is_empty(),
        "docs/handoffs/2026-09-13-databricks-findings.md does not name every model excluded \
         from a sweep: {missing:?}"
    );
}

/// `docs/specs/multi_backend.md` §Known Divergences no longer claims the
/// Databricks capability matrix column is unverified — phases 4c-9f measured
/// it live — and it cites this handoff as the evidence.
#[test]
fn the_spec_no_longer_claims_the_capability_column_is_unverified() {
    let spec_path = repo_root().join("docs/specs/multi_backend.md");
    let spec = fs::read_to_string(&spec_path).unwrap_or_else(|e| panic!("read {spec_path:?}: {e}"));

    assert!(
        !spec.contains("inherited, not independently verified"),
        "docs/specs/multi_backend.md still claims the Databricks capability matrix column is \
         unverified, but phases 4c-9f measured it live against a real workspace"
    );
    assert!(
        spec.contains("2026-09-13-databricks-findings.md"),
        "docs/specs/multi_backend.md's Databricks entries do not cite \
         docs/handoffs/2026-09-13-databricks-findings.md as the evidence"
    );
}
