//! Classification of an oracle failure: did the backend reject *this SQL*, or
//! is the oracle itself unusable?
//!
//! Split out of `smelt-db`'s `prop_helpers/oracle_check.rs` when the oracle
//! transport was promoted into this crate. The comparison logic that consumes
//! it (`check_types_against_oracle`) stayed behind, because it depends on
//! `smelt-db`'s type inference and on that crate's generators.

/// Whether an oracle failure means "the backend understood this SQL and
/// rejected it" — safe to skip, exactly like the existing DuckDB/Spark legs
/// already do for dialect gaps the generators occasionally produce — or "the
/// oracle itself is unusable," which must fail the test loudly rather than
/// silently pass every case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OracleErrorKind {
    /// The backend parsed/analysed the request and rejected this specific
    /// SQL (unknown function, bad cast target, syntax error, missing table).
    /// Skipping is correct: the generators sometimes produce SQL a given
    /// backend legitimately doesn't accept.
    QueryRefusal,
    /// The oracle can't be trusted to answer at all right now — auth/token
    /// failure, a dead subprocess, a lock/IO error. Continuing would make the
    /// whole leg report green while testing nothing.
    Fatal,
}

/// Classify an oracle failure string as a refusal of this query (skip) or a
/// broken oracle (fail loud).
///
/// This is an **allow-list**, not a deny-list: `Fatal` is the default for
/// anything unrecognised (fail-loud discipline — see root `CLAUDE.md`
/// §"Fail-loud discipline"). That default matters concretely for BigQuery: a
/// live probe against a real warehouse found that an expired/malformed
/// access token does *not* surface as HTTP 401/403 — it surfaces as a
/// client-side google-auth message with no HTTP status in it at all
/// ("The credentials do not contain the necessary fields need to refresh
/// the access token."). A classifier built as a 401/403 deny-list would
/// silently swallow exactly the failure this function exists to catch. Only
/// the explicit refusal shapes below are treated as skippable; 401/403 stay
/// recognised as Fatal defensively (a revoked-permission case may still
/// produce one), and every other unrecognised 4xx/5xx or transport error
/// falls through to Fatal.
pub fn classify_oracle_error(msg: &str) -> OracleErrorKind {
    if is_recognized_query_refusal(msg) {
        OracleErrorKind::QueryRefusal
    } else {
        OracleErrorKind::Fatal
    }
}

/// The explicit, verified set of "backend understood and rejected this SQL"
/// shapes. Every pattern here was confirmed against a real backend (DuckDB
/// locally; BigQuery via a live dry-run probe), not guessed from
/// documentation.
fn is_recognized_query_refusal(msg: &str) -> bool {
    // DuckDB: binder/parser/catalog/conversion errors are the backend's own
    // rejection of the SQL, not an oracle malfunction. Verified against a
    // real DuckDB: `SELECT nosuchfunction(1)` -> "Catalog Error: Scalar
    // Function with name nosuchfunction does not exist!"; `SELECT 1 +` ->
    // "Parser Error: syntax error at end of input"; `CAST(1 AS FOOBAR)` ->
    // "Catalog Error: Type with name FOOBAR does not exist!". Binder/
    // Conversion Error are the same family for type-mismatch rejections
    // (e.g. the `percentile_ordered_set_decimal` divergence's neighbours).
    const DUCKDB_REFUSALS: &[&str] = &[
        "Catalog Error",
        "Parser Error",
        "Binder Error",
        "Conversion Error",
    ];
    if DUCKDB_REFUSALS.iter().any(|p| msg.contains(p)) {
        return true;
    }

    // Spark: `spark_oracle.rs` already collapses AnalysisException/
    // ParseException/"Error in query" from the session output into this
    // fixed message before it ever reaches here; the two exception names are
    // matched too in case a future caller passes raw session output through
    // unfiltered.
    if msg.contains("spark-sql error in output")
        || msg.contains("AnalysisException")
        || msg.contains("ParseException")
    {
        return true;
    }

    // BigQuery: a dry-run job the warehouse itself rejected as bad SQL.
    // Verified against a live warehouse (see the BigQuery backend handoff
    // probe): `SELECT nosuchfunction(1)` -> "400 POST
    // https://bigquery.googleapis.com/.../jobs?prettyPrint=false: Function
    // not found: nosuchfunction at [1:8] ..."; an empty select list -> "400
    // POST .../jobs?...: Syntax error: SELECT list must not be empty ...";
    // a missing table -> "404 POST .../jobs?...: Not found: Table ...". Only
    // 400/404 against the jobs-submission endpoint count as a refusal —
    // deliberately narrow, so a 401/403 permission failure or a 500
    // transient error is never absorbed here (it falls through to Fatal).
    let trimmed = msg.trim_start();
    for status in ["400 POST ", "404 POST "] {
        if let Some(rest) = trimmed.strip_prefix(status) {
            if rest.contains("bigquery.googleapis.com") {
                return true;
            }
        }
    }

    // BigQuery, *execution* rather than job submission. The dry-run path above
    // fails at submission and carries the endpoint URL; a real execution fails
    // after the job is accepted and carries none, so the URL check cannot see
    // it. Verified against a live warehouse by the dialect-audit value leg:
    // `POWER(-2.5, -2.5)` -> "400 Floating point error in function: POW(...);
    // reason: invalidQuery"; `ARRAY_AGG` over a NULL-bearing group -> "400
    // Array cannot have a null element ...; reason: invalidQuery"; an
    // unsupported analytic function -> "400 Analytic function
    // APPROX_COUNT_DISTINCT is not supported.; reason: invalidQuery".
    //
    // `reason: invalidQuery` is the discriminator, and it is deliberately
    // narrow: BigQuery tags a *client-side query* problem that way, and tags
    // auth, quota, and backend failures with other reasons — which therefore
    // still fall through to Fatal, as fail-loud discipline requires.
    if trimmed.starts_with("400 ") && msg.contains("reason: invalidQuery") {
        return true;
    }

    false
}

#[cfg(test)]
mod classify_oracle_error_tests {
    use super::*;

    /// Table-driven over representative message strings, most captured
    /// verbatim from a live probe (a real DuckDB locally, a real BigQuery
    /// warehouse via a dry-run oracle probe) rather than guessed — so the
    /// classifier is verified against the actual shapes it will see, not an
    /// idealized version of them.
    fn cases() -> Vec<(&'static str, OracleErrorKind)> {
        use OracleErrorKind::{Fatal, QueryRefusal};
        vec![
            // --- BigQuery *execution* refusals, captured verbatim from the
            // dialect-audit value leg against a live warehouse.
            (
                "400 Floating point error in function: POW(-2.5, -2.5); reason: invalidQuery, location: query",
                QueryRefusal,
            ),
            (
                "400 Array cannot have a null element; error in writing field p_array_agg_agg; reason: invalidQuery",
                QueryRefusal,
            ),
            (
                "400 Analytic function APPROX_COUNT_DISTINCT is not supported.; reason: invalidQuery, location: query",
                QueryRefusal,
            ),
            // …but a 400 whose reason is NOT a query problem stays Fatal: an
            // expired credential must never read as "this SQL was rejected".
            ("400 Request had invalid authentication credentials; reason: authError", Fatal),
            ("400 Quota exceeded; reason: quotaExceeded", Fatal),
            // --- DuckDB refusals (verified: `SELECT nosuchfunction(1)`,
            // `SELECT 1 +`, `CAST(1 AS FOOBAR)` against a real DuckDB) ---
            (
                "prepare: Catalog Error: Scalar Function with name nosuchfunction does not exist!\n\
                 Did you mean \"countif\"?\n\nLINE 1: SELECT nosuchfunction(1)\n               ^",
                QueryRefusal,
            ),
            ("prepare: Parser Error: syntax error at end of input", QueryRefusal),
            (
                "prepare: Catalog Error: Type with name FOOBAR does not exist!\nDid you mean \"JSON\"?",
                QueryRefusal,
            ),
            (
                "prepare: Binder Error: No function matches the given name and argument types",
                QueryRefusal,
            ),
            (
                "query: Conversion Error: Could not cast value 74.260000 to DECIMAL(2,1)",
                QueryRefusal,
            ),
            // --- Spark refusals ---
            ("spark-sql error in output", QueryRefusal),
            ("AnalysisException: cannot resolve 'x'", QueryRefusal),
            // --- BigQuery refusals (verified via a live dry-run probe) ---
            (
                "400 POST https://bigquery.googleapis.com/bigquery/v2/projects/smelt-bq-test-20260816/jobs?prettyPrint=false: Function not found: nosuchfunction at [1:8]\n\nLocation: US\nJob ID: dda7f018-...\n",
                QueryRefusal,
            ),
            (
                "400 POST https://bigquery.googleapis.com/bigquery/v2/projects/smelt-bq-test-20260816/jobs?prettyPrint=false: Syntax error: SELECT list must not be empty at [1:8]\n\nLocation: US\nJob ID: ...\n",
                QueryRefusal,
            ),
            (
                "404 POST https://bigquery.googleapis.com/bigquery/v2/projects/smelt-bq-test-20260816/jobs?prettyPrint=false: Not found: Table smelt-bq-test-20260816:smelt_test.no_such_table_xyz was not found in location US\n\nLocation: US\nJob ID: ...\n",
                QueryRefusal,
            ),
            // --- Fatal: the load-bearing case. A live probe found an
            // expired/malformed BigQuery access token does NOT surface as
            // 401/403 — it's a client-side google-auth message with no HTTP
            // status at all. A 401/403-only deny-list would miss this. ---
            (
                "The credentials do not contain the necessary fields need to refresh the access token. \
                 You must specify refresh_token, token_uri, client_id, and client_secret.",
                Fatal,
            ),
            // --- Fatal: empty-token startup failure / child death ---
            (
                "bigquery type oracle startup failed: BigQuery requires an explicit OAuth access token \
                 (SMELT_BQ_ACCESS_TOKEN). smelt never falls back to Google application-default \
                 credentials: ...",
                Fatal,
            ),
            ("bigquery type oracle exited", Fatal),
            // --- Fatal: transport/process-death shapes from bigquery_oracle.rs
            // and spark_oracle.rs ---
            ("lock: poisoned lock", Fatal),
            ("write request: broken pipe", Fatal),
            ("flush: broken pipe", Fatal),
            ("read reply: unexpected EOF", Fatal),
            ("bad reply \"garbage\": expected value", Fatal),
            ("spark-sql EOF", Fatal),
            ("read: connection reset", Fatal),
            // --- Fatal: kept defensively even though the live probe shows a
            // bad token doesn't take this shape — a revoked-permission case
            // may still produce a plain 401/403. ---
            ("401 Unauthorized: invalid credentials", Fatal),
            (
                "403 POST https://bigquery.googleapis.com/bigquery/v2/projects/x/jobs: Access Denied",
                Fatal,
            ),
            ("invalid_grant: Token has been expired or revoked.", Fatal),
            // --- Fatal by default: unrecognised messages must not be
            // silently treated as refusals (fail-loud discipline). A 500 from
            // BigQuery's job endpoint is deliberately NOT in the refusal
            // list — it means the warehouse itself is unhappy, not that this
            // SQL was rejected. ---
            (
                "500 POST https://bigquery.googleapis.com/bigquery/v2/projects/x/jobs: Internal error",
                Fatal,
            ),
            ("some completely unrecognised error string", Fatal),
        ]
    }

    #[test]
    fn classifies_representative_messages() {
        for (msg, expected) in cases() {
            let actual = classify_oracle_error(msg);
            assert_eq!(
                actual, expected,
                "classify_oracle_error({msg:?}) = {actual:?}, expected {expected:?}"
            );
        }
    }
}
