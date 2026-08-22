//! Token preflight and quota-refusal classification for BigQuery maintenance-
//! conformance sweeps (`docs/plans/20260817-bigquery-generative-conformance.md`
//! Phase 3).
//!
//! Two independent failure modes threaten a live BigQuery sweep, and both are
//! classified explicitly rather than absorbed:
//!
//! * **Token exhaustion mid-sweep.** `scripts/bigquery-auth.sh` mints a
//!   short-lived (1 hour) token and writes it, plus a unix-epoch expiry
//!   stamp, to `$SMELT_BQ_CONFIG_DIR/token` (default
//!   `$HOME/.config/gcloud-smelt-bq/token`). [`preflight`] reads that stamp
//!   and refuses to start a sweep it cannot finish inside the remaining
//!   window, rather than discovering the expiry mid-run as an opaque
//!   failure.
//! - **Per-table modification rate limiting.** BigQuery refuses a burst of
//!   modifications to one table with a recognisable "quota for table update
//!   operations" shape. [`classify_bq_error`] tells that shape apart from an
//!   auth failure and from everything else, and [`retry_with_backoff`] is
//!   the only path allowed to retry — bounded, and failing loud on
//!   exhaustion.
//!
//! ## Fail-loud discipline (root `CLAUDE.md` §"Fail-loud discipline")
//!
//! [`classify_bq_error`] is an **allow-list**, not a deny-list:
//! [`BqErrorClass::Other`] is the default for anything unrecognised. This
//! mirrors `crates/smelt-db/tests/prop_helpers/oracle_check.rs`'s
//! `classify_oracle_error`, which exists for the identical reason and
//! already paid for the load-bearing lesson this module reuses: a live
//! probe against a real BigQuery warehouse found that an expired/malformed
//! access token does **not** surface as HTTP 401/403 at all — it is a
//! client-side google-auth message with no HTTP status in it whatsoever
//! ("The credentials do not contain the necessary fields need to refresh
//! the access token. You must specify refresh_token, token_uri, client_id,
//! and client_secret."), captured verbatim in that file's
//! `classify_oracle_error` test table. A classifier built as a 401/403
//! deny-list would silently swallow exactly the failure this module exists
//! to catch, so unrecognised errors — including any HTTP status this module
//! does not explicitly know — fall through to `Other`, and every `Other`
//! and every `Auth` result fails the calling leg rather than being
//! retried or skipped.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Per-step pacing floor between consecutive modifications of one BigQuery
/// table.
///
/// Measured, not guessed: `docs/research/20260816-bigquery-backend.md`
/// §"Measured against the live warehouse" ran `scripts/bigquery-probe-quota.sh`
/// against a live warehouse on 2026-08-17 (`smelt-bq-test-20260816`, US),
/// holding the interval between consecutive *request starts* so the
/// request's own round trip never contributes to the spacing. 2 s spacing
/// was refused with the table-update-quota shape (7/8 succeeded, refused at
/// op 6); 3 s was not (10/10 succeeded). 3 s is therefore the measured
/// floor at which the burst limit stops binding.
pub const TABLE_MODIFICATION_PACING: Duration = Duration::from_secs(3);

/// Bound on retry attempts for a [`BqErrorClass::QuotaRetryable`] failure
/// before [`retry_with_backoff`] fails the leg. Retry must be bounded per
/// the fail-loud discipline — an unbounded retry loop would just move the
/// silent-hang failure mode from "converted to a skip" to "never returns".
pub const MAX_QUOTA_RETRIES: u32 = 5;

/// How a BigQuery-facing failure classifies for retry/preflight purposes.
///
/// This is deliberately a three-way split, not a boolean: a quota refusal
/// is retryable, an auth failure never is (retrying can't fix an expired
/// token), and everything else is an unrecognised failure that must fail
/// the leg rather than being guessed at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BqErrorClass {
    /// The `quota for table update operations` / `too many table update
    /// operations` burst-limit shape — safe to retry with backoff, bounded
    /// by [`MAX_QUOTA_RETRIES`].
    QuotaRetryable,
    /// A recognised auth/credential failure — including the client-side,
    /// no-HTTP-status expired-token message — never retryable.
    Auth,
    /// Anything not on the allow-list above. The default; never retried,
    /// never treated as a skip.
    Other,
}

/// Classify a BigQuery-facing error message.
///
/// An allow-list, not a deny-list — see the module-level fail-loud
/// discussion. Every pattern here is a shape verified against a real
/// warehouse (via `scripts/bigquery-probe-quota.sh` for the quota shape, and
/// via the live probe recorded in `crates/smelt-db/tests/prop_helpers/oracle_check.rs`
/// for the auth shape), not guessed from documentation.
pub fn classify_bq_error(msg: &str) -> BqErrorClass {
    if is_quota_refusal(msg) {
        BqErrorClass::QuotaRetryable
    } else if is_auth_failure(msg) {
        BqErrorClass::Auth
    } else {
        BqErrorClass::Other
    }
}

/// The table-update-quota burst-limit shape.
///
/// Verified against a live warehouse by `scripts/bigquery-probe-quota.sh`,
/// which itself only accepts these two exact substrings as the refusal it
/// is measuring (see its `msg == *"exceeded quota for table update
/// operations"*` / `*"Exceeded rate limits: too many table update
/// operations"*` check) — kept in sync with that script rather than
/// invented independently.
fn is_quota_refusal(msg: &str) -> bool {
    const QUOTA_SHAPES: &[&str] = &[
        "exceeded quota for table update operations",
        "Exceeded rate limits: too many table update operations",
    ];
    QUOTA_SHAPES.iter().any(|p| msg.contains(p))
}

/// Recognised auth/credential failure shapes.
///
/// The load-bearing entry is the first one: an expired/malformed BigQuery
/// access token surfaces as this exact client-side google-auth message with
/// no HTTP status at all (captured verbatim from the live probe recorded in
/// `oracle_check.rs`). 401/403 are kept as a defensive fallback — a
/// revoked-permission case may still produce a plain HTTP 401/403 — but the
/// no-status shape is the one that actually occurs for an expired token, so
/// it cannot be reached by a 401/403-only deny-list.
fn is_auth_failure(msg: &str) -> bool {
    const AUTH_SHAPES: &[&str] = &[
        "The credentials do not contain the necessary fields need to refresh the access token",
        "invalid_grant",
        "Reauthentication is needed",
        "401 Unauthorized",
    ];
    if AUTH_SHAPES.iter().any(|p| msg.contains(p)) {
        return true;
    }
    let trimmed = msg.trim_start();
    trimmed.starts_with("401 ") || trimmed.starts_with("403 ")
}

/// Default location of the token file `scripts/bigquery-auth.sh` writes:
/// `$SMELT_BQ_CONFIG_DIR/token`, falling back to
/// `$HOME/.config/gcloud-smelt-bq/token` to match that script's own
/// `BQ_CONFIG_DIR` default.
pub fn default_token_path() -> PathBuf {
    let dir = std::env::var("SMELT_BQ_CONFIG_DIR").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_default();
        format!("{home}/.config/gcloud-smelt-bq")
    });
    PathBuf::from(dir).join("token")
}

/// Compute the remaining lifetime of the token at `token_path`.
///
/// Reads the two-line token file `bigquery-auth.sh` writes (line 1: token,
/// line 2: unix-epoch expiry) and fails if:
/// - the file is absent or unreadable (never assume a good window with no
///   evidence of one);
/// - the expiry line is missing or does not parse as a unix timestamp;
/// - the token has already expired.
///
/// Pulled out of [`preflight`] so both the per-test check and the
/// once-per-process sweep check ([`preflight_sweep_once`]) share the exact
/// same window arithmetic and the exact same failure messages for an
/// absent/malformed/expired token — only the "is the window long enough"
/// comparison differs between the two callers.
fn remaining_window(token_path: impl AsRef<Path>) -> Result<Duration, String> {
    let path = token_path.as_ref();
    let contents = fs::read_to_string(path).map_err(|e| {
        format!(
            "no usable BigQuery token at {}: {e} — run scripts/bigquery-auth.sh before starting this sweep",
            path.display()
        )
    })?;

    let mut lines = contents.lines();
    let token = lines.next().unwrap_or("");
    if token.trim().is_empty() {
        return Err(format!(
            "token file at {} has no token on its first line — run scripts/bigquery-auth.sh",
            path.display()
        ));
    }

    let expiry_line = lines.next().ok_or_else(|| {
        format!(
            "token file at {} has no expiry stamp on its second line — run scripts/bigquery-auth.sh",
            path.display()
        )
    })?;
    let expiry_line = expiry_line.trim();
    let expires_at: u64 = expiry_line.parse().map_err(|e| {
        format!(
            "unparseable expiry stamp {expiry_line:?} in {}: {e} — run scripts/bigquery-auth.sh",
            path.display()
        )
    })?;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| format!("system clock is before the unix epoch: {e}"))?
        .as_secs();

    let remaining_secs = expires_at.checked_sub(now).ok_or_else(|| {
        format!(
            "BigQuery token at {} already expired {} s ago — run scripts/bigquery-auth.sh",
            path.display(),
            now.saturating_sub(expires_at)
        )
    })?;

    Ok(Duration::from_secs(remaining_secs))
}

/// Env var overriding [`DEFAULT_SWEEP_ESTIMATE_SECS`]. See
/// [`resolve_sweep_estimate`].
pub const SWEEP_ESTIMATE_ENV_VAR: &str = "SMELT_CONFORMANCE_BQ_SWEEP_ESTIMATE_SECS";

/// Default sweep-cost estimate, in seconds, used by [`preflight_sweep_once`]
/// when [`SWEEP_ESTIMATE_ENV_VAR`] is unset.
///
/// Derivation: the last measured all-green **sequential** run of the full
/// BigQuery maintenance-conformance sweep (21 tests, `--test-threads=1`)
/// took 2190.85 s (2026-08-21). 2700 s (45 min) carries roughly 500 s of
/// headroom over that measurement while still comfortably fitting inside the
/// ~1 h (~3500 s usable) token window `scripts/bigquery-auth.sh` mints.
///
/// This is deliberately a **sequential**-cost ceiling, not a
/// concurrency-adjusted one: `preflight_sweep_once` is checked once per
/// process regardless of how many threads `--test-threads` gives the
/// runner, so treating it as the sum of the sweep's own (sequential) work
/// keeps it a safe upper bound no matter what concurrency the runner
/// chooses — running the same sweep concurrently only makes the real
/// wall-clock cost *lower* than this estimate, never higher.
///
/// If any `SMELT_CONFORMANCE_BQ_*_CASES` env var is raised above its
/// default (more generated cases per family), the sweep's real sequential
/// cost goes up too, and this estimate no longer bounds it — that is
/// exactly why it is overridable via [`SWEEP_ESTIMATE_ENV_VAR`] rather than
/// hardcoded.
///
/// For scale: the same suite measured **621.61 s** all-green at the script's
/// default 4-way concurrency (22 tests, 2026-08-22), so this ceiling is
/// roughly four times a real concurrent sweep. That asymmetry is deliberate.
/// The two ways to be wrong here are not equally cheap: over-refusing costs
/// a human one extra passphrase entry, while under-refusing burns real
/// warehouse quota on a sweep that dies mid-case and proves nothing about
/// the cases it never reached. A sequential ceiling is the bound that stays
/// correct if someone drops back to `--test-threads=1` to debug.
pub const DEFAULT_SWEEP_ESTIMATE_SECS: u64 = 2700;

/// Pure resolution of the sweep estimate from an optional env-var value.
///
/// `None` (the env var unset) resolves to [`DEFAULT_SWEEP_ESTIMATE_SECS`].
/// `Some(s)` must parse as a `u64` or this is a loud error — per the
/// fail-loud discipline (root `CLAUDE.md` §"Fail-loud discipline"), an
/// unparseable override must never be silently absorbed into the default,
/// which would hide a typo behind a value that looks like it did something.
///
/// Takes the env var's value as a parameter rather than reading
/// `std::env::var` internally so it can be tested without mutating
/// process-global environment state, which would race against every other
/// test in this binary reading the same variable — the same convention
/// `backend.rs`'s `skip_reason_for_project` documents and uses.
pub fn resolve_sweep_estimate(env_value: Option<&str>) -> Result<Duration, String> {
    match env_value {
        None => Ok(Duration::from_secs(DEFAULT_SWEEP_ESTIMATE_SECS)),
        Some(raw) => raw.parse::<u64>().map(Duration::from_secs).map_err(|e| {
            format!("{SWEEP_ESTIMATE_ENV_VAR}={raw:?} is not a valid number of seconds: {e}")
        }),
    }
}

/// Non-memoized sweep-level preflight check: does the token at `token_path`
/// have at least `sweep_estimate` of remaining life?
///
/// Pulled out from [`preflight_sweep_once`] so the check logic itself can be
/// unit-tested directly, without going through the `OnceLock` memoization
/// (which is inherently a once-per-process effect and not something a test
/// can exercise more than once per test binary).
fn check_sweep_preflight(
    token_path: impl AsRef<Path>,
    sweep_estimate: Duration,
) -> Result<(), String> {
    let path = token_path.as_ref();
    let remaining = remaining_window(path)?;

    if remaining < sweep_estimate {
        let shortfall = sweep_estimate - remaining;
        return Err(format!(
            "BigQuery token window ({remaining:?} remaining) is shorter than the whole sweep's \
             estimated duration ({sweep_estimate:?}) — shortfall {shortfall:?}; run \
             scripts/bigquery-auth.sh again before starting"
        ));
    }

    Ok(())
}

/// Sweep-level token preflight, evaluated **once per process** and memoized.
///
/// [`preflight`] answers "can THIS test's estimated cost fit in the
/// remaining window?" — the right question when tests run one at a time
/// (`--test-threads=1`), but the wrong one once tests run concurrently:
/// every `#[test]` would independently pass its own small per-test budget
/// while the sweep as a whole still needed far more than the window, and the
/// token could expire mid-sweep with no test ever having been warned. The
/// unit of budgeting has to be the SWEEP, not the individual test — one
/// process-wide check against the sweep's total estimated cost
/// ([`resolve_sweep_estimate`]), performed exactly once regardless of how
/// many `#[test]` wrappers call this function or how many threads run them
/// concurrently.
///
/// `OnceLock::get_or_init` guarantees exactly one evaluation even when many
/// test threads call this concurrently and race to be first — every caller,
/// first or not, gets a clone of the same `Result`.
pub fn preflight_sweep_once(
    token_path: impl AsRef<Path>,
    sweep_estimate: Duration,
) -> Result<(), String> {
    static RESULT: OnceLock<Result<(), String>> = OnceLock::new();
    RESULT
        .get_or_init(|| check_sweep_preflight(token_path, sweep_estimate))
        .clone()
}

/// Block the calling thread for `delay`. A named hook (rather than an
/// inline `std::thread::sleep` at every call site) so a spacing constant
/// such as [`TABLE_MODIFICATION_PACING`] always reads as "pace by the
/// measured floor" at the call site.
pub fn pace(delay: Duration) {
    std::thread::sleep(delay);
}

/// Run `attempt` up to `max_retries + 1` times, retrying only on
/// [`BqErrorClass::QuotaRetryable`] failures with `backoff` between
/// attempts (via [`pace`]).
///
/// - A non-quota failure (`Auth` or `Other`) returns immediately — retrying
///   an auth failure or an unrecognised error can't help and would just
///   hide it longer.
/// - Exhausting `max_retries` on a persistent quota refusal is a failure,
///   never a skip: the caller gets `Err`, never a value that pretends the
///   step succeeded.
pub fn retry_with_backoff<T>(
    mut attempt: impl FnMut(u32) -> Result<T, String>,
    max_retries: u32,
    backoff: Duration,
) -> Result<T, String> {
    let mut last_err: Option<String> = None;
    for n in 0..=max_retries {
        match attempt(n) {
            Ok(v) => return Ok(v),
            Err(e) => match classify_bq_error(&e) {
                BqErrorClass::QuotaRetryable => {
                    let exhausted = n == max_retries;
                    last_err = Some(e);
                    if exhausted {
                        break;
                    }
                    pace(backoff);
                }
                BqErrorClass::Auth | BqErrorClass::Other => return Err(e),
            },
        }
    }
    Err(format!(
        "quota retry exhausted after {} attempt(s): {}",
        max_retries + 1,
        last_err.unwrap_or_default()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn write_token_file(dir: &std::path::Path, body: &str) -> PathBuf {
        let path = dir.join("token");
        let mut f = fs::File::create(&path).expect("create token file");
        f.write_all(body.as_bytes()).expect("write token file");
        path
    }

    fn now_secs() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before epoch")
            .as_secs()
    }

    #[test]
    fn remaining_window_reports_the_time_left_on_a_live_stamp() {
        let dir = tempfile::tempdir().expect("tempdir");
        let expires = now_secs() + 3600;
        let path = write_token_file(dir.path(), &format!("faketoken123\n{expires}\n"));

        let remaining = remaining_window(&path).expect("a future stamp must yield a window");
        // Wall-clock may advance a second between the stamp and the read.
        assert!(
            remaining <= Duration::from_secs(3600) && remaining >= Duration::from_secs(3600 - 5),
            "expected ~3600s remaining, got {remaining:?}"
        );
    }

    /// The token-FILE failure modes — absent, truncated, unparseable,
    /// already expired — asserted against [`remaining_window`], the single
    /// owner of that arithmetic and of these messages. Both the sweep
    /// preflight and any future caller inherit them from here, so they are
    /// pinned once rather than per caller.
    #[test]
    fn remaining_window_refuses_an_absent_or_unparseable_expiry_stamp() {
        let dir = tempfile::tempdir().expect("tempdir");

        // Absent file entirely.
        let missing = dir.path().join("no-such-token");
        let err = remaining_window(&missing)
            .expect_err("a missing token file must never be treated as a good window");
        assert!(err.contains("no usable BigQuery token"), "got: {err}");

        // Present file, but no second (expiry) line at all.
        let no_expiry_path = write_token_file(dir.path(), "faketoken123\n");
        let err = remaining_window(&no_expiry_path)
            .expect_err("a token file with no expiry line must refuse, not assume a good window");
        assert!(err.contains("expiry stamp"), "got: {err}");

        // Present file, unparseable expiry line.
        let garbage_path = write_token_file(dir.path(), "faketoken123\nnot-a-timestamp\n");
        let err = remaining_window(&garbage_path)
            .expect_err("an unparseable expiry stamp must refuse, not assume a good window");
        assert!(err.contains("unparseable expiry stamp"), "got: {err}");

        // Present file, already-expired stamp.
        let expired_path = write_token_file(
            dir.path(),
            &format!("faketoken123\n{}\n", now_secs().saturating_sub(100)),
        );
        let err =
            remaining_window(&expired_path).expect_err("an already-expired stamp must refuse");
        assert!(err.contains("already expired"), "got: {err}");
    }

    #[test]
    fn expired_token_is_classified_despite_having_no_http_status() {
        // The exact client-side google-auth message a live probe found an
        // expired/malformed BigQuery token actually produces — captured
        // verbatim in crates/smelt-db/tests/prop_helpers/oracle_check.rs's
        // classify_oracle_error test table. No "401", no "403", no HTTP
        // status token anywhere in the string.
        let msg = "The credentials do not contain the necessary fields need to refresh the access \
                    token. You must specify refresh_token, token_uri, client_id, and client_secret.";
        assert!(!msg.contains("401"));
        assert!(!msg.contains("403"));
        assert_eq!(classify_bq_error(msg), BqErrorClass::Auth);

        // invalid_grant is the other client-side shape google-auth produces
        // for a revoked/expired token — also no HTTP status.
        let msg2 = "invalid_grant: Token has been expired or revoked.";
        assert_eq!(classify_bq_error(msg2), BqErrorClass::Auth);

        // Plain 401/403 stay classified as Auth too (defensive: a
        // revoked-permission case may still produce one).
        assert_eq!(
            classify_bq_error("401 Unauthorized: invalid credentials"),
            BqErrorClass::Auth
        );
        assert_eq!(
            classify_bq_error(
                "403 POST https://bigquery.googleapis.com/bigquery/v2/projects/x/jobs: Access Denied"
            ),
            BqErrorClass::Auth
        );
    }

    #[test]
    fn quota_refusal_is_allow_listed_and_everything_else_fails() {
        assert_eq!(
            classify_bq_error(
                "Job exceeded rate limits: Your table exceeded quota for table update operations"
            ),
            BqErrorClass::QuotaRetryable
        );
        assert_eq!(
            classify_bq_error("Exceeded rate limits: too many table update operations"),
            BqErrorClass::QuotaRetryable
        );

        // An unrecognised 4xx is NOT retryable — it falls through to Other,
        // never absorbed as a quota refusal.
        assert_eq!(
            classify_bq_error(
                "400 POST https://bigquery.googleapis.com/bigquery/v2/projects/x/jobs: Syntax error"
            ),
            BqErrorClass::Other
        );
        // A 500 is deliberately not treated as retryable quota either — it
        // means the warehouse itself is unhappy, a different failure class
        // this module does not claim to handle.
        assert_eq!(
            classify_bq_error(
                "500 POST https://bigquery.googleapis.com/bigquery/v2/projects/x/jobs: Internal error"
            ),
            BqErrorClass::Other
        );
        assert_eq!(
            classify_bq_error("some completely unrecognised error string"),
            BqErrorClass::Other
        );

        // retry_with_backoff actually honours the classification: a quota
        // error is retried, a non-quota 4xx returns immediately without
        // consuming a retry.
        let mut calls = 0u32;
        let result: Result<(), String> = retry_with_backoff(
            |_n| {
                calls += 1;
                Err(
                    "400 POST https://bigquery.googleapis.com/bigquery/v2/projects/x/jobs: bad"
                        .into(),
                )
            },
            5,
            Duration::from_millis(1),
        );
        assert!(result.is_err());
        assert_eq!(calls, 1, "a non-quota failure must not be retried");
    }

    #[test]
    fn sweep_estimate_defaults_when_env_var_unset() {
        assert_eq!(
            resolve_sweep_estimate(None).expect("default must resolve"),
            Duration::from_secs(DEFAULT_SWEEP_ESTIMATE_SECS)
        );
    }

    #[test]
    fn sweep_estimate_honours_a_valid_override() {
        assert_eq!(
            resolve_sweep_estimate(Some("1200")).expect("valid override must resolve"),
            Duration::from_secs(1200)
        );
    }

    #[test]
    fn sweep_estimate_refuses_an_unparseable_override_loudly() {
        let err = resolve_sweep_estimate(Some("not-a-number"))
            .expect_err("an unparseable override must be a loud error, not a silent default");
        assert!(
            err.contains(SWEEP_ESTIMATE_ENV_VAR),
            "error should name the env var, got: {err}"
        );
        assert!(
            err.contains("not-a-number"),
            "error should name the bad value, got: {err}"
        );
    }

    #[test]
    fn sweep_preflight_refuses_a_window_shorter_than_the_sweep_estimate() {
        let dir = tempfile::tempdir().expect("tempdir");
        let expires = now_secs() + 100;
        let path = write_token_file(dir.path(), &format!("faketoken123\n{expires}\n"));

        let err = check_sweep_preflight(&path, Duration::from_secs(2700))
            .expect_err("a 100s window must refuse a 2700s sweep estimate");
        assert!(
            err.contains("shortfall"),
            "expected the refusal to name the shortfall, got: {err}"
        );
    }

    #[test]
    fn sweep_preflight_accepts_an_ample_window() {
        let dir = tempfile::tempdir().expect("tempdir");
        let expires = now_secs() + 3600;
        let path = write_token_file(dir.path(), &format!("faketoken123\n{expires}\n"));

        check_sweep_preflight(&path, Duration::from_secs(2700))
            .expect("a 1h window must accept a 2700s sweep estimate");
    }

    #[test]
    fn retry_exhaustion_fails_the_leg() {
        let mut calls = 0u32;
        let result: Result<(), String> = retry_with_backoff(
            |_n| {
                calls += 1;
                Err("Job exceeded rate limits: Your table exceeded quota for table update operations"
                    .to_string())
            },
            3,
            Duration::from_millis(1),
        );

        let err = result.expect_err("persistent quota refusal must fail the leg, never skip");
        assert!(
            err.contains("exhausted"),
            "expected the failure to say retries were exhausted, got: {err}"
        );
        assert_eq!(
            calls, 4,
            "expected max_retries + 1 attempts (3 retries after the first try)"
        );

        // A quota failure that clears before exhaustion succeeds and stops
        // retrying immediately.
        let mut calls2 = 0u32;
        let result2: Result<&'static str, String> = retry_with_backoff(
            |n| {
                calls2 += 1;
                if n < 2 {
                    Err("exceeded quota for table update operations".to_string())
                } else {
                    Ok("ok")
                }
            },
            5,
            Duration::from_millis(1),
        );
        assert_eq!(result2, Ok("ok"));
        assert_eq!(calls2, 3);
    }
}
