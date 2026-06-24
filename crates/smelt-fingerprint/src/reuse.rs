//! Reuse-condition evaluator for virtual environments (D-46, D-47).
//!
//! Given the current model's fingerprint and a candidate snapshot entry's source
//! SQL, decides whether the existing physical table may be reused instead of
//! rebuilding. All four conditions from `virtual_environments.md` §"Semantics /
//! Reuse decision" are evaluated in order; the function is pure (no I/O) so it
//! is trivially unit-testable and callable from any context.
//!
//! Condition 4 (schema migration) is **stubbed** — it always passes and sets
//! `ReuseOutcome::schema_migration_checked = false`. Full implementation depends
//! on `schema_evolution.md` work (not yet scaffolded).

use smelt_core::config::StateMode;
use smelt_core::metadata::ReuseConfig;

use crate::{output_fingerprint_from_sql, Fingerprint, FingerprintResult};

/// Parameters for the reuse-condition evaluator.
pub struct ReuseParams {
    /// Effective state mode for this model (pre-computed as the minimum of the
    /// project mode and any per-model narrowing). Only `StateMode::Environments`
    /// passes condition 1.
    pub effective_mode: StateMode,
    /// Pre-computed fingerprint of the current model's expanded SQL.
    pub current_fingerprint: Fingerprint,
    /// SQL text from the candidate `SnapshotEntry`. The evaluator recomputes the
    /// fingerprint fresh to avoid false positives from compiler-version drift.
    pub candidate_source_sql: String,
    /// Output schema used when recomputing the candidate fingerprint (may be empty).
    pub candidate_output_schema: Vec<(String, String)>,
    /// Full fingerprint result for the current model (provides `deterministic`).
    pub fingerprint_result: FingerprintResult,
    /// Author reuse-override hatches from model frontmatter (`accept_current`,
    /// `assert_deterministic`). `None` means neither hatch is set.
    pub model_reuse: Option<ReuseConfig>,
}

/// Outcome of the reuse-condition check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReuseDecision {
    /// All conditions passed — the existing physical table may be reused.
    Reuse(ReuseOutcome),
    /// At least one condition failed — the model must be rebuilt.
    Rebuild(Vec<ReuseConditionFailed>),
}

/// Details of an approved reuse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReuseOutcome {
    /// Which reuse contract applies.
    pub path: ReusePath,
    /// Trust note logged when an author hatch was the deciding factor for
    /// condition 3. `None` when the model was natively deterministic.
    pub trust_note: Option<TrustNote>,
    /// Whether condition 4 (schema migration) was fully evaluated. Always
    /// `false` until the `schema_evolution.md` work lands.
    pub schema_migration_checked: bool,
}

/// The reuse contract strength.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReusePath {
    /// Model is deterministic (or asserted so via `assert_deterministic`).
    /// Rebuilding would produce an identical relation; the existing table is
    /// proven equivalent.
    RebuildIdentical,
    /// Model is non-deterministic; `accept_current` was set by the author.
    /// The existing table is preserved without a rebuild — it may differ from
    /// a fresh run. Output-preserving contract, not rebuild-identity.
    OutputPreserving,
}

/// Trust note emitted when an author hatch was the deciding factor for condition 3.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrustNote {
    /// `assert_deterministic: true` was trusted (condition 3a override). The
    /// inline non-determinism detector flagged the model, but the author asserted
    /// determinism; that assertion is trusted and logged.
    AssertDeterministicTrusted,
    /// `accept_current: true` was applied (condition 3b). Non-deterministic model;
    /// the author accepted output-preserving reuse.
    AcceptCurrentApplied,
}

/// Reason reuse was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReuseConditionFailed {
    /// Condition 1: effective state mode is not `environments`. Snapshot reuse
    /// requires the project (and model, after narrowing) to be in
    /// `StateMode::Environments`.
    NotEnvironmentMode,
    /// Condition 2: the current fingerprint does not equal the fingerprint
    /// recomputed from the candidate's `source_sql`. The model's SQL has changed,
    /// or the candidate SQL is unparseable.
    FingerprintMismatch,
    /// Condition 3: the model is non-deterministic and neither `assert_deterministic`
    /// nor `accept_current` is set in the model's frontmatter.
    NeitherReuseHatchSet,
}

/// Evaluate whether the existing physical table for a model may be reused under
/// the virtual-environments policy (`virtual_environments.md` §"Semantics / Reuse
/// decision"; D-46, D-47).
///
/// The four conditions are checked in order; the first failure returns immediately.
/// The function is pure (no I/O) — every input is passed explicitly.
///
/// # Condition 4 stub
///
/// Condition 4 (schema migration check) is not yet implemented. All reuse outcomes
/// carry `schema_migration_checked: false` to make the stub visible to callers.
pub fn evaluate_reuse(params: ReuseParams) -> ReuseDecision {
    // ── Condition 1: effective mode must be Environments ─────────────────────
    if params.effective_mode != StateMode::Environments {
        return ReuseDecision::Rebuild(vec![ReuseConditionFailed::NotEnvironmentMode]);
    }

    // ── Condition 2: fingerprint(current) == fingerprint(candidate.source_sql) ─
    // Recompute fresh from source SQL to avoid false positives from
    // compiler-version drift — never compare stored fingerprint_hex fields.
    let candidate_fp = output_fingerprint_from_sql(
        &params.candidate_source_sql,
        &params.candidate_output_schema,
    );
    let candidate_fingerprint = match candidate_fp {
        Some(r) => r.fingerprint,
        None => return ReuseDecision::Rebuild(vec![ReuseConditionFailed::FingerprintMismatch]),
    };
    if params.current_fingerprint != candidate_fingerprint {
        return ReuseDecision::Rebuild(vec![ReuseConditionFailed::FingerprintMismatch]);
    }

    // ── Condition 3: determinism check + author hatches ───────────────────────
    let assert_det = params
        .model_reuse
        .as_ref()
        .map(|r| r.assert_deterministic)
        .unwrap_or(false);
    let accept_current = params
        .model_reuse
        .as_ref()
        .map(|r| r.accept_current)
        .unwrap_or(false);

    // 3a: natively deterministic model — no trust note needed.
    if params.fingerprint_result.deterministic {
        return ReuseDecision::Reuse(ReuseOutcome {
            path: ReusePath::RebuildIdentical,
            trust_note: None,
            schema_migration_checked: false,
        });
    }

    // 3a (override): assert_deterministic hatch. When both assert_deterministic
    // and accept_current are set, 3a wins — rebuild-identity is the stronger
    // contract (spec-silent tie-break; documented in plan P4 test notes).
    if assert_det {
        return ReuseDecision::Reuse(ReuseOutcome {
            path: ReusePath::RebuildIdentical,
            trust_note: Some(TrustNote::AssertDeterministicTrusted),
            schema_migration_checked: false,
        });
    }

    // 3b: accept_current hatch — output-preserving reuse. Trust note always set.
    if accept_current {
        return ReuseDecision::Reuse(ReuseOutcome {
            path: ReusePath::OutputPreserving,
            trust_note: Some(TrustNote::AcceptCurrentApplied),
            schema_migration_checked: false,
        });
    }

    // Condition 3 failed: non-deterministic and no hatch set.
    // Condition 4 is a stub that always passes; it is only reachable when
    // conditions 1–3 pass, so it is implicit in the Reuse return values above.
    ReuseDecision::Rebuild(vec![ReuseConditionFailed::NeitherReuseHatchSet])
}
