//! Reuse-condition evaluator tests — P4 of W8 virtual_env.
//!
//! Covers all four conditions (condition 4 is stubbed) plus edge cases for the
//! logged-trust notes and the 3a/3b tie-break. See
//! `docs/plans/20260620-w8-virtual-env.md` §"Phase P4".

use smelt_core::config::StateMode;
use smelt_core::metadata::ReuseConfig;
use smelt_fingerprint::{
    output_fingerprint_from_sql,
    reuse::{
        evaluate_reuse, ReuseConditionFailed, ReuseDecision, ReuseOutcome, ReuseParams, ReusePath,
        TrustNote,
    },
};

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Simple deterministic SQL suitable for fingerprint tests.
const DET_SQL: &str = "SELECT a FROM (SELECT 1 AS a) AS t";
/// Simple non-deterministic SQL (inline `random()` call).
const NONDET_SQL: &str = "SELECT random() AS r FROM (SELECT 1 AS a) AS t";
/// A different deterministic SQL (produces a different fingerprint from DET_SQL).
const ALT_SQL: &str = "SELECT b FROM (SELECT 2 AS b) AS t";

fn det_params() -> ReuseParams {
    let fp = output_fingerprint_from_sql(DET_SQL, &[]).expect("DET_SQL parses");
    ReuseParams {
        effective_mode: StateMode::Environments,
        current_fingerprint: fp.fingerprint,
        candidate_source_sql: DET_SQL.to_string(),
        candidate_output_schema: vec![],
        fingerprint_result: fp,
        model_reuse: None,
    }
}

fn nondet_params() -> ReuseParams {
    let fp = output_fingerprint_from_sql(NONDET_SQL, &[]).expect("NONDET_SQL parses");
    ReuseParams {
        effective_mode: StateMode::Environments,
        current_fingerprint: fp.fingerprint,
        candidate_source_sql: NONDET_SQL.to_string(),
        candidate_output_schema: vec![],
        fingerprint_result: fp,
        model_reuse: None,
    }
}

// ── Condition 1: effective mode must be Environments ──────────────────────────

#[test]
fn cond1_fails_stateless() {
    let mut p = det_params();
    p.effective_mode = StateMode::Stateless;
    match evaluate_reuse(p) {
        ReuseDecision::Rebuild(reasons) => {
            assert_eq!(reasons, vec![ReuseConditionFailed::NotEnvironmentMode]);
        }
        ReuseDecision::Reuse(_) => panic!("expected Rebuild"),
    }
}

#[test]
fn cond1_fails_intervals_mode() {
    // `intervals` does not enable snapshot reuse — only `environments` does.
    let mut p = det_params();
    p.effective_mode = StateMode::Intervals;
    match evaluate_reuse(p) {
        ReuseDecision::Rebuild(reasons) => {
            assert_eq!(reasons, vec![ReuseConditionFailed::NotEnvironmentMode]);
        }
        ReuseDecision::Reuse(_) => panic!("expected Rebuild"),
    }
}

#[test]
fn cond1_fails_model_narrowed_to_stateless() {
    // A model narrowed to `stateless` in an `environments` project → effective_mode = Stateless.
    // The caller computes effective_mode; here we just pass Stateless.
    let mut p = det_params();
    p.effective_mode = StateMode::Stateless;
    assert!(matches!(
        evaluate_reuse(p),
        ReuseDecision::Rebuild(ref r) if r == &[ReuseConditionFailed::NotEnvironmentMode]
    ));
}

// ── Condition 2: fingerprint match ────────────────────────────────────────────

#[test]
fn cond2_passes_when_fingerprints_match() {
    // Same SQL → same fingerprint → condition 2 passes.
    let p = det_params();
    // With a deterministic model, reuse should be approved.
    assert!(matches!(
        evaluate_reuse(p),
        ReuseDecision::Reuse(ReuseOutcome {
            path: ReusePath::RebuildIdentical,
            ..
        })
    ));
}

#[test]
fn cond2_fails_fingerprint_mismatch() {
    // current_fingerprint is for DET_SQL, but candidate has ALT_SQL → mismatch.
    let fp = output_fingerprint_from_sql(DET_SQL, &[]).expect("parses");
    let p = ReuseParams {
        effective_mode: StateMode::Environments,
        current_fingerprint: fp.fingerprint,
        candidate_source_sql: ALT_SQL.to_string(),
        candidate_output_schema: vec![],
        fingerprint_result: fp,
        model_reuse: None,
    };
    match evaluate_reuse(p) {
        ReuseDecision::Rebuild(reasons) => {
            assert_eq!(reasons, vec![ReuseConditionFailed::FingerprintMismatch]);
        }
        ReuseDecision::Reuse(_) => panic!("expected Rebuild"),
    }
}

#[test]
fn cond2_fails_when_candidate_sql_unparseable() {
    let fp = output_fingerprint_from_sql(DET_SQL, &[]).expect("parses");
    let p = ReuseParams {
        effective_mode: StateMode::Environments,
        current_fingerprint: fp.fingerprint,
        candidate_source_sql: "NOT VALID SQL AT ALL".to_string(),
        candidate_output_schema: vec![],
        fingerprint_result: fp,
        model_reuse: None,
    };
    match evaluate_reuse(p) {
        ReuseDecision::Rebuild(reasons) => {
            assert_eq!(reasons, vec![ReuseConditionFailed::FingerprintMismatch]);
        }
        ReuseDecision::Reuse(_) => panic!("expected Rebuild"),
    }
}

// ── Condition 3: determinism + hatches ───────────────────────────────────────

#[test]
fn cond3_deterministic_no_override_gives_rebuild_identical_no_trust_note() {
    let p = det_params();
    match evaluate_reuse(p) {
        ReuseDecision::Reuse(outcome) => {
            assert_eq!(outcome.path, ReusePath::RebuildIdentical);
            assert!(outcome.trust_note.is_none(), "no trust note expected");
        }
        ReuseDecision::Rebuild(_) => panic!("expected Reuse"),
    }
}

#[test]
fn cond3a_nondeterministic_with_assert_deterministic_gives_rebuild_identical() {
    let mut p = nondet_params();
    p.model_reuse = Some(ReuseConfig {
        assert_deterministic: true,
        accept_current: false,
    });
    match evaluate_reuse(p) {
        ReuseDecision::Reuse(outcome) => {
            assert_eq!(outcome.path, ReusePath::RebuildIdentical);
            assert_eq!(
                outcome.trust_note,
                Some(TrustNote::AssertDeterministicTrusted)
            );
        }
        ReuseDecision::Rebuild(_) => panic!("expected Reuse"),
    }
}

#[test]
fn cond3b_nondeterministic_with_accept_current_gives_output_preserving() {
    let mut p = nondet_params();
    p.model_reuse = Some(ReuseConfig {
        assert_deterministic: false,
        accept_current: true,
    });
    match evaluate_reuse(p) {
        ReuseDecision::Reuse(outcome) => {
            assert_eq!(outcome.path, ReusePath::OutputPreserving);
            assert_eq!(outcome.trust_note, Some(TrustNote::AcceptCurrentApplied));
        }
        ReuseDecision::Rebuild(_) => panic!("expected Reuse"),
    }
}

#[test]
fn cond3_fails_nondeterministic_no_hatch() {
    let p = nondet_params();
    match evaluate_reuse(p) {
        ReuseDecision::Rebuild(reasons) => {
            assert_eq!(reasons, vec![ReuseConditionFailed::NeitherReuseHatchSet]);
        }
        ReuseDecision::Reuse(_) => panic!("expected Rebuild"),
    }
}

#[test]
fn cond3_both_hatches_prefers_3a_rebuild_identical() {
    // When both assert_deterministic and accept_current are set on a non-deterministic
    // model, condition 3a wins — rebuild-identity is the stronger contract.
    let mut p = nondet_params();
    p.model_reuse = Some(ReuseConfig {
        assert_deterministic: true,
        accept_current: true,
    });
    match evaluate_reuse(p) {
        ReuseDecision::Reuse(outcome) => {
            assert_eq!(outcome.path, ReusePath::RebuildIdentical);
            assert_eq!(
                outcome.trust_note,
                Some(TrustNote::AssertDeterministicTrusted)
            );
        }
        ReuseDecision::Rebuild(_) => panic!("expected Reuse"),
    }
}

// ── Condition 4: stub ─────────────────────────────────────────────────────────

#[test]
fn cond4_stub_schema_migration_checked_false() {
    // Condition 4 is stubbed: always passes, schema_migration_checked == false.
    let p = det_params();
    match evaluate_reuse(p) {
        ReuseDecision::Reuse(outcome) => {
            assert!(!outcome.schema_migration_checked, "stub: not yet checked");
        }
        ReuseDecision::Rebuild(_) => panic!("expected Reuse"),
    }
}

// ── Full happy path ───────────────────────────────────────────────────────────

#[test]
fn happy_path_deterministic_model_all_conditions_pass() {
    let p = det_params();
    match evaluate_reuse(p) {
        ReuseDecision::Reuse(outcome) => {
            assert_eq!(outcome.path, ReusePath::RebuildIdentical);
            assert!(outcome.trust_note.is_none());
            assert!(!outcome.schema_migration_checked);
        }
        ReuseDecision::Rebuild(_) => panic!("expected Reuse"),
    }
}
