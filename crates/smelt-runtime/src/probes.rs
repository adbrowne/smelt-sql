//! Single-owner probe dispatch — the run driver's one place that decides
//! *whether* a declared-fact probe runs this run and, if it fires, builds
//! the named diagnostic (`docs/specs/model_properties.md` §"Probe
//! obligation", §"Probe cadence"). The cadence *decision* is pure
//! (`smelt_logical::maintenance::probe_cadence::should_dispatch`) — this
//! module adds the backend execution, the shared `violation_count`/
//! `sample_keys` row contract (`model_properties.md` §"Probe obligation"),
//! and the firing message shared by every probe consumer.

use std::collections::BTreeMap;

use smelt_backend::{Backend, BackendError};
use smelt_core::config::ProbeCadence;
use smelt_logical::maintenance::{should_dispatch, ProbeDispatch, SkipReason};

/// The cadence policy plus this model's run ordinal (its prior-run count —
/// 0 for its first run), resolved once per run in `execute.rs` and threaded
/// to every probe dispatch site.
#[derive(Debug, Clone, Copy)]
pub struct ProbePolicy {
    pub cadence: ProbeCadence,
    pub run_ordinal: u64,
}

impl ProbePolicy {
    pub fn new(cadence: ProbeCadence, run_ordinal: u64) -> Self {
        ProbePolicy {
            cadence,
            run_ordinal,
        }
    }

    /// Convenience for callers (and tests) that only care about
    /// `per_run` — the default cadence.
    pub fn per_run() -> Self {
        ProbePolicy {
            cadence: ProbeCadence::PerRun,
            run_ordinal: 0,
        }
    }
}

/// The declaration, licensed maintenance cell, and remedy a firing probe's
/// diagnostic names (`model_properties.md` §"Probe obligation": "a firing
/// probe's diagnostic carries the violated fact, the maintenance cell it
/// was licensing, and the remedy text from the registry").
#[derive(Debug, Clone)]
pub struct ProbeContext {
    /// The registry's named diagnostic code, e.g. `KeyedRecurrenceBoundViolated`.
    pub probe_code: String,
    /// The declared fact being verified, e.g. `key_recurrence`.
    pub fact: String,
    pub model: String,
    /// The maintenance cell/technique this probe licenses.
    pub cell: String,
    pub remedy: String,
}

/// One row of a dispatched probe's result set.
pub type ProbeRow = BTreeMap<String, String>;

/// The outcome of a probe dispatch decision.
#[derive(Debug, Clone)]
pub enum ProbeVerdict {
    /// Cadence policy skipped this dispatch — the declaration is trusted
    /// and recorded unverified on the run.
    Skipped(SkipReason),
    /// The probe ran and found no violation.
    Held,
    /// The probe ran and found `count` violating row(s).
    Violated { count: u64, sample_keys: String },
}

/// Parse the shared probe row contract: one row, `violation_count` (`0` =
/// holds), `sample_keys` (optional, empty when holding). Fails loud on a
/// missing/unparseable `violation_count` — a malformed probe row refuses
/// rather than being read as "no violation" (`CLAUDE.md` §"Fail-loud
/// discipline").
pub fn parse_violation_count_contract(
    probe_code: &str,
    model: &str,
    rows: &[ProbeRow],
) -> Result<(u64, String), BackendError> {
    let violation_count: u64 = rows
        .first()
        .and_then(|r| r.get("violation_count"))
        .ok_or_else(|| BackendError::ExecutionFailed {
            model: model.to_string(),
            message: format!(
                "{probe_code} probe for model '{model}' returned no `violation_count` row — \
                 refusing to trust an unchecked result"
            ),
        })?
        .parse::<u64>()
        .map_err(|e| BackendError::ExecutionFailed {
            model: model.to_string(),
            message: format!(
                "{probe_code} probe for model '{model}' returned an unparseable \
                 `violation_count`: {e}"
            ),
        })?;
    let sample_keys = rows
        .first()
        .and_then(|r| r.get("sample_keys"))
        .cloned()
        .unwrap_or_default();
    Ok((violation_count, sample_keys))
}

/// Dispatch one probe: consult cadence, and — only if dispatching — execute
/// `sql` against `backend` and parse the shared `violation_count`/
/// `sample_keys` contract. Callers whose probe SQL speaks a different row
/// shape (e.g. the count-preservation probe's `driving_count`/
/// `enriched_count`) consult [`should_dispatch`] directly instead of this
/// helper, but still share [`probe_violation_suffix`] for the firing
/// message.
pub async fn dispatch_probe(
    backend: &dyn Backend,
    policy: &ProbePolicy,
    ctx: &ProbeContext,
    sql: &str,
) -> Result<ProbeVerdict, BackendError> {
    match should_dispatch(policy.cadence, policy.run_ordinal) {
        ProbeDispatch::Skip(reason) => return Ok(ProbeVerdict::Skipped(reason)),
        ProbeDispatch::Dispatch => {}
    }
    let batches = backend
        .execute_sql(sql)
        .await
        .map_err(|e| BackendError::ExecutionFailed {
            model: ctx.model.clone(),
            message: format!(
                "Failed to execute {} probe for model '{}':\n  SQL: {}\n  Error: {}",
                ctx.probe_code, ctx.model, sql, e
            ),
        })?;
    let rows = crate::check_runner::batches_to_rows(&batches);
    let (count, sample_keys) = parse_violation_count_contract(&ctx.probe_code, &ctx.model, &rows)?;
    if count == 0 {
        Ok(ProbeVerdict::Held)
    } else {
        Ok(ProbeVerdict::Violated { count, sample_keys })
    }
}

/// The shared trailer a firing probe's diagnostic appends to its
/// probe-specific message prefix: names the licensed cell and the remedy
/// (`model_properties.md` §"Probe obligation").
pub fn probe_violation_suffix(ctx: &ProbeContext) -> String {
    format!(" Licensed cell: {}. Remedy: {}.", ctx.cell, ctx.remedy)
}
