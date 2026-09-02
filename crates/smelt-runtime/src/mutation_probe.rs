//! Mutation-happened discrimination for a live `UpstreamMutation` cell
//! (`docs/specs/incremental_models.md` §"When a mutation cell dispatches"):
//! before a run dispatches a technique licensed by an `UpstreamMutation`
//! trigger, it compares the source's recorded whole-source content
//! fingerprint against the source's CURRENT whole-source state. An
//! unchanged source (same row count, same digest-column set, same content
//! fingerprint) means the cell is recorded as a no-op this run and no
//! maintenance statement for it executes; anything else means the cell
//! dispatches as it always has, and the observed fingerprint becomes the
//! new baseline.
//!
//! This module owns the pure verdict ([`decide_mutation_dispatch`]) and the
//! backend-executing helper ([`probe_source_mutation_fingerprint`]) that
//! reads the source's current fingerprint via
//! [`smelt_logical::maintenance::emit::emit_source_mutation_fingerprint`] —
//! the single emitter authoring that statement (`CLAUDE.md`
//! §"Maintenance-plan purity" — statement emission single owner). Call
//! sites in `execute.rs` own persisting the refreshed baseline (via
//! `smelt_state::source_mutations::SourceMutationStore`), same division of
//! responsibility as `source_probes.rs`.

use smelt_backend::{Backend, BackendError, MaintenanceDialect};
use smelt_logical::maintenance::emit::emit_source_mutation_fingerprint;
use smelt_state::source_mutations::SourceMutationBaseline;

/// A source's observed whole-source fingerprint this run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedSourceFingerprint {
    pub count: i64,
    pub fingerprint: String,
}

/// Whether an otherwise-live `UpstreamMutation` cell dispatches this run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MutationVerdict {
    /// The source's fingerprint differs from the recorded baseline (or no
    /// baseline is recorded yet, or the digest-column set changed) — the
    /// cell dispatches and the observed fingerprint is re-recorded.
    Dispatch,
    /// The source's fingerprint exactly matches the recorded baseline over
    /// the same digest-column set — the cell is a no-op this run.
    NoOp,
}

/// Pure verdict: compare `baseline` (`None` when nothing has been recorded
/// for this source yet) against `observed`, gated by whether
/// `digest_columns` still matches the recorded baseline's own digest-column
/// set. A digest-column-set mismatch makes the recorded value incomparable
/// to `observed` — that always dispatches (never a silent skip), matching
/// `docs/specs/incremental_models.md` §"When a mutation cell dispatches".
pub fn decide_mutation_dispatch(
    baseline: Option<&SourceMutationBaseline>,
    digest_columns: &[String],
    observed: &ObservedSourceFingerprint,
) -> MutationVerdict {
    let Some(baseline) = baseline else {
        return MutationVerdict::Dispatch;
    };
    if baseline.digest_columns != digest_columns {
        return MutationVerdict::Dispatch;
    }
    if baseline.recorded_count != observed.count
        || baseline.recorded_fingerprint != observed.fingerprint
    {
        return MutationVerdict::Dispatch;
    }
    MutationVerdict::NoOp
}

/// Execute `emit_source_mutation_fingerprint`'s statement against `backend`
/// and parse the single-row `current_count`/`current_fingerprint` result
/// into an [`ObservedSourceFingerprint`].
///
/// `source_table` is already fully qualified (`schema.table`).
pub async fn probe_source_mutation_fingerprint(
    backend: &dyn Backend,
    model: &str,
    source_address: &str,
    source_table: &str,
    digest_columns: &[String],
    dialect: MaintenanceDialect,
) -> Result<ObservedSourceFingerprint, BackendError> {
    let stmt = emit_source_mutation_fingerprint(source_table, digest_columns, dialect);
    let batches =
        backend
            .execute_sql(&stmt.sql)
            .await
            .map_err(|e| BackendError::ExecutionFailed {
                model: model.to_string(),
                message: format!(
                    "Failed to execute source-mutation fingerprint for source '{}' (model \
                     '{}'):\n  SQL: {}\n  Error: {}",
                    source_address, model, stmt.sql, e
                ),
            })?;
    let rows = crate::check_runner::batches_to_rows(&batches);
    let row = rows.first().ok_or_else(|| BackendError::ExecutionFailed {
        model: model.to_string(),
        message: format!(
            "Source-mutation fingerprint for source '{}' (model '{}') returned no rows",
            source_address, model
        ),
    })?;
    let count: i64 = row
        .get("current_count")
        .ok_or_else(|| BackendError::ExecutionFailed {
            model: model.to_string(),
            message: format!(
                "Source-mutation fingerprint for source '{}' (model '{}') returned no \
                 `current_count`",
                source_address, model
            ),
        })?
        .parse()
        .map_err(|e| BackendError::ExecutionFailed {
            model: model.to_string(),
            message: format!(
                "Source-mutation fingerprint for source '{}' (model '{}') returned an \
                 unparseable `current_count`: {e}",
                source_address, model
            ),
        })?;
    let fingerprint = row.get("current_fingerprint").cloned().unwrap_or_default();
    Ok(ObservedSourceFingerprint { count, fingerprint })
}

/// The full gate: probe `source_table`'s current fingerprint, decide against
/// `baseline`, and — on [`MutationVerdict::Dispatch`] — return the refreshed
/// baseline the caller should record AFTER the licensed technique's write
/// actually succeeds (never before: a failed run must not suppress the next
/// run's cell, `docs/specs/incremental_models.md` §"When a mutation cell
/// dispatches").
pub async fn gate_upstream_mutation_dispatch(
    backend: &dyn Backend,
    model: &str,
    source_address: &str,
    source_table: &str,
    digest_columns: &[String],
    dialect: MaintenanceDialect,
    baseline: Option<&SourceMutationBaseline>,
) -> Result<(MutationVerdict, SourceMutationBaseline), BackendError> {
    let observed = probe_source_mutation_fingerprint(
        backend,
        model,
        source_address,
        source_table,
        digest_columns,
        dialect,
    )
    .await?;
    let verdict = decide_mutation_dispatch(baseline, digest_columns, &observed);
    let refreshed = SourceMutationBaseline {
        recorded_count: observed.count,
        recorded_fingerprint: observed.fingerprint,
        digest_columns: digest_columns.to_vec(),
    };
    Ok((verdict, refreshed))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cols(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn dispatch_when_no_baseline() {
        let observed = ObservedSourceFingerprint {
            count: 10,
            fingerprint: "abc".to_string(),
        };
        let verdict = decide_mutation_dispatch(None, &cols(&["id"]), &observed);
        assert_eq!(verdict, MutationVerdict::Dispatch);
    }

    #[test]
    fn noop_when_count_and_fingerprint_unchanged() {
        let baseline = SourceMutationBaseline {
            recorded_count: 10,
            recorded_fingerprint: "abc".to_string(),
            digest_columns: cols(&["id"]),
        };
        let observed = ObservedSourceFingerprint {
            count: 10,
            fingerprint: "abc".to_string(),
        };
        let verdict = decide_mutation_dispatch(Some(&baseline), &cols(&["id"]), &observed);
        assert_eq!(verdict, MutationVerdict::NoOp);
    }

    #[test]
    fn dispatch_when_fingerprint_changed() {
        let baseline = SourceMutationBaseline {
            recorded_count: 10,
            recorded_fingerprint: "abc".to_string(),
            digest_columns: cols(&["id"]),
        };
        let observed = ObservedSourceFingerprint {
            count: 10,
            fingerprint: "different".to_string(),
        };
        let verdict = decide_mutation_dispatch(Some(&baseline), &cols(&["id"]), &observed);
        assert_eq!(verdict, MutationVerdict::Dispatch);
    }

    #[test]
    fn dispatch_when_count_changed() {
        let baseline = SourceMutationBaseline {
            recorded_count: 10,
            recorded_fingerprint: "abc".to_string(),
            digest_columns: cols(&["id"]),
        };
        let observed = ObservedSourceFingerprint {
            count: 11,
            fingerprint: "abc".to_string(),
        };
        let verdict = decide_mutation_dispatch(Some(&baseline), &cols(&["id"]), &observed);
        assert_eq!(verdict, MutationVerdict::Dispatch);
    }

    #[test]
    fn dispatch_when_digest_column_set_changed() {
        let baseline = SourceMutationBaseline {
            recorded_count: 10,
            recorded_fingerprint: "abc".to_string(),
            digest_columns: cols(&["id"]),
        };
        // Same count/fingerprint values, but the digest-column set the
        // caller wants to compare under has changed — incomparable, so this
        // must dispatch rather than silently trusting a baseline recorded
        // under a different column set.
        let observed = ObservedSourceFingerprint {
            count: 10,
            fingerprint: "abc".to_string(),
        };
        let verdict =
            decide_mutation_dispatch(Some(&baseline), &cols(&["id", "amount"]), &observed);
        assert_eq!(verdict, MutationVerdict::Dispatch);
    }
}
