use std::time::Duration as StdDuration;

use anyhow::Result;

use smelt_backend::BackendError;

use crate::reporter::RunReporter;
use crate::types::ExecuteRequest;

/// Default retry bound (`ExecuteRequest::retry_max`) and base backoff, in
/// milliseconds (`ExecuteRequest::retry_backoff_ms`), used when a request
/// leaves either field unset (`docs/plans/20260719-prod-w2-operability.md`
/// Phase 6).
pub(crate) const DEFAULT_RETRY_MAX: u32 = 3;
pub(crate) const DEFAULT_RETRY_BACKOFF_MS: u64 = 200;

/// Deterministic backoff delay for retry `attempt` (1-based) of
/// `model_name` within run `run_id`: exponential backoff off
/// `base_backoff_ms`, jittered by a stable hash of `(run_id, model_name,
/// attempt)` — never real-clock entropy (`rand`, `Instant`, `SystemTime`),
/// so retry timing is reproducible and tests never race a real delay
/// (`docs/plans/20260719-prod-w2-operability.md` Phase 6: "jitter from
/// run_id hash — no `Date::now` coupling in tests").
pub(crate) fn retry_backoff_delay(
    base_backoff_ms: u64,
    attempt: u32,
    run_id: &str,
    model_name: &str,
) -> StdDuration {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    // Cap the shift so a pathologically high `retry_max` cannot overflow.
    let shift = attempt.saturating_sub(1).min(16);
    let exponential = base_backoff_ms.saturating_mul(1u64 << shift);
    let mut hasher = DefaultHasher::new();
    (run_id, model_name, attempt).hash(&mut hasher);
    let jitter = if base_backoff_ms == 0 {
        0
    } else {
        hasher.finish() % base_backoff_ms
    };
    StdDuration::from_millis(exponential.saturating_add(jitter))
}

/// Resolved retry policy for one model's maintenance write
/// (`docs/plans/20260719-prod-w2-operability.md` Phase 6). Carries exactly
/// the fields [`retry_backend_call`] needs so every statement-group-issuing
/// call site — the incremental/full-refresh dispatch in this module AND the
/// column-scoped-MERGE (MP11), T3 delta-restricted DeleteInsert, and
/// windowed-keyed-maintenance (`refresh: keyed`) dispatch in
/// `maintenance_driver.rs`/`cumulative.rs` — retries a transient backend
/// error identically, rather than each layer growing its own copy of the
/// backoff/jitter math. `retry_max: 0` (an operator's `retry_max: 0` request,
/// or a test that does not exercise retry) makes every retry-guarded call a
/// single, unretried attempt — behaviourally identical to no retry wrapper
/// at all.
pub struct RetryPolicy<'a> {
    pub retry_max: u32,
    pub base_backoff_ms: u64,
    pub run_id: &'a str,
    pub model_name: &'a str,
    pub reporter: &'a dyn RunReporter,
}

impl<'a> RetryPolicy<'a> {
    /// Resolve a request's `retry_max`/`retry_backoff_ms` (falling back to
    /// [`DEFAULT_RETRY_MAX`]/[`DEFAULT_RETRY_BACKOFF_MS`]) into a policy for
    /// `model_name` within `run_id`.
    pub fn from_request(
        request: &ExecuteRequest,
        run_id: &'a str,
        model_name: &'a str,
        reporter: &'a dyn RunReporter,
    ) -> Self {
        Self {
            retry_max: request.retry_max.unwrap_or(DEFAULT_RETRY_MAX),
            base_backoff_ms: request.retry_backoff_ms.unwrap_or(DEFAULT_RETRY_BACKOFF_MS),
            run_id,
            model_name,
            reporter,
        }
    }
}

/// Bounded retry with exponential backoff wrapping a single backend call
/// whose whole effect is safe to re-issue on a transient failure — one
/// model's *whole* statement group (drop+create for a full refresh, or one
/// batch's DELETE+INSERT/MERGE/APPEND), or a maintenance helper that reads a
/// fact then issues exactly one such statement group (T3 delta-restricted
/// DeleteInsert, MP11 column-scoped MERGE, `refresh: keyed`'s
/// create-or-merge write) — never a
/// partial slice of it, and never an earlier, already-succeeded statement
/// group belonging to the same model
/// (`docs/plans/20260719-prod-w2-operability.md` Phase 6, review checklist
/// "no partial-write replay hazard"). Each of this function's call sites in
/// `execute_one_model` passes a closure that re-invokes exactly one such
/// backend call; the closure itself is idempotent-safe to re-run because it
/// starts with `DROP ... IF EXISTS` (full refresh) or is a backend-native
/// transactional DELETE+INSERT/MERGE (incremental) —
/// `Backend::delete_and_insert_transactional`'s own contract
/// (`crates/smelt-backend/src/lib.rs`) already guarantees a failed INSERT
/// rolls back its DELETE, so retrying re-applies the same transaction
/// rather than compounding a partial write.
///
/// Retries only `BackendError::is_transient` failures; a deterministic
/// SQL/type/constraint error is returned to the caller on the first
/// attempt, unretried.
pub(crate) async fn retry_backend_call<T, F, Fut>(
    policy: &RetryPolicy<'_>,
    mut call: F,
) -> Result<T, BackendError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, BackendError>>,
{
    let mut attempt: u32 = 0;
    loop {
        match call().await {
            Ok(value) => return Ok(value),
            Err(err) if attempt < policy.retry_max && err.is_transient() => {
                attempt += 1;
                policy.reporter.model_retrying(
                    policy.run_id,
                    policy.model_name,
                    attempt,
                    policy.retry_max,
                    &err.to_string(),
                );
                let delay = retry_backoff_delay(
                    policy.base_backoff_ms,
                    attempt,
                    policy.run_id,
                    policy.model_name,
                );
                if !delay.is_zero() {
                    tokio::time::sleep(delay).await;
                }
            }
            Err(err) => return Err(err),
        }
    }
}

/// Convenience wrapper over [`retry_backend_call`] for this module's own
/// call sites, which already hold an [`ExecuteRequest`] rather than a
/// pre-resolved [`RetryPolicy`].
pub(crate) async fn retry_statement_group<T, F, Fut>(
    request: &ExecuteRequest,
    run_id: &str,
    model_name: &str,
    reporter: &dyn RunReporter,
    call: F,
) -> Result<T, BackendError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, BackendError>>,
{
    let policy = RetryPolicy::from_request(request, run_id, model_name, reporter);
    retry_backend_call(&policy, call).await
}
