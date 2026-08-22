//! Shared "run smelt inference, compare against a live oracle" logic used by
//! every dual-target (DuckDB/Spark) type property test.
//!
//! Factored out of `type_property_tests.rs` so `prop_numeric_function_types.rs`
//! (and any future targeted property test) can reuse the same comparison path
//! instead of re-implementing it.

use std::collections::HashMap;

use smelt_db::type_inference::{infer_select_column_types, TypeContext};
use smelt_parser::ast::File;
use smelt_types::{DataType, TypedColumn};

use super::divergences::{find_divergence, TypeDivergence};
use super::duckdb_oracle::TypeOracle;
use super::generators::{TypedExpr, TypedSource};
use super::known_unknowns::{find_known_unknown, KnownUnknown};
use super::type_comparison::{compare_types, TypeMatch};

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

    false
}

/// What `check_types_against_oracle` actually verified for one generated
/// case. Exists so a caller can accumulate coverage across many cases and
/// assert a floor (see the BigQuery leg in `type_property_tests.rs`) — a leg
/// that "ran" every case but compared zero columns, because every case was
/// skipped as a refusal, is indistinguishable from a healthy run unless
/// something counts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OracleCheckOutcome {
    /// Number of output columns whose smelt-inferred type was actually
    /// compared against the oracle's reported type this call (includes
    /// known-unknown and registered-divergence matches — anything that
    /// reached the comparison logic, not just exact matches).
    pub columns_compared: usize,
    /// True when the whole case was skipped because the oracle refused the
    /// generated SQL outright — comparison never started.
    pub query_refused: bool,
}

/// Parse SQL with smelt and run type inference on each select column.
pub fn run_smelt_inference(sql: &str, columns: &[TypedSource]) -> Vec<(String, DataType)> {
    let parse = smelt_parser::parse(sql);
    let root = parse.syntax();
    let file = File::cast(root).expect("failed to cast to File");
    let select_stmt = file.select_stmt().expect("no SelectStmt in parsed SQL");

    let mut ctx = TypeContext::new();
    for col in columns {
        ctx.add_cte_column(
            "data",
            &col.name,
            TypedColumn::nullable(col.data_type.clone()),
        );
    }

    let column_types = infer_select_column_types(&select_stmt, &ctx);

    let select_list = select_stmt.select_list().expect("no select list");
    let items: Vec<_> = select_list.items().collect();

    items
        .iter()
        .zip(column_types.iter())
        .map(|(item, typed_col)| {
            let alias = item.alias().unwrap_or_else(|| "?".to_string());
            (alias, typed_col.data_type.clone())
        })
        .collect()
}

/// Build an alias → generating-SQL lookup so an inferred `Unknown` column can be
/// matched against the known-unknowns registry by its generating expression.
pub fn expr_sql_by_alias(exprs: &[TypedExpr]) -> HashMap<String, String> {
    exprs
        .iter()
        .map(|e| (e.alias.clone(), e.sql.clone()))
        .collect()
}

/// Compare smelt inference against one oracle backend, returning an error message on mismatch.
///
/// Returns `Err` both for a genuine type mismatch and for a `Fatal` oracle
/// failure (see `classify_oracle_error`) — either way the caller's `if let
/// Err(msg) = ...` sites fail the test loudly. A `QueryRefusal` oracle
/// failure returns `Ok` with `query_refused: true` and zero columns
/// compared, matching the previous "skip invalid SQL for this backend"
/// behaviour, but now the skip is visible to the caller instead of silent.
#[allow(clippy::too_many_arguments)]
pub fn check_types_against_oracle(
    oracle: &dyn TypeOracle,
    backend: &str,
    sql: &str,
    columns: &[TypedSource],
    exprs: &[TypedExpr],
    divergences: &[TypeDivergence],
    unknowns: &[KnownUnknown],
) -> Result<OracleCheckOutcome, String> {
    let actual_types = match oracle.query_types(sql) {
        Ok(types) => types,
        Err(msg) => {
            return match classify_oracle_error(&msg) {
                OracleErrorKind::QueryRefusal => Ok(OracleCheckOutcome {
                    columns_compared: 0,
                    query_refused: true,
                }),
                OracleErrorKind::Fatal => Err(format!(
                    "{backend} oracle unusable — treating as fatal, not a query refusal \
                     (see classify_oracle_error):\n  {msg}\n  SQL: {sql}"
                )),
            };
        }
    };

    let inferred_types = run_smelt_inference(sql, columns);
    let by_alias = expr_sql_by_alias(exprs);

    let mut columns_compared = 0usize;
    for (i, actual) in actual_types.iter().enumerate() {
        let inferred = if i < inferred_types.len() {
            &inferred_types[i]
        } else {
            continue;
        };
        columns_compared += 1;

        let smelt_type = &inferred.1;
        let actual_type = &actual.1;

        if smelt_type.is_unknown() {
            let expr_sql = by_alias.get(&inferred.0).map(String::as_str).unwrap_or(sql);
            if find_known_unknown(expr_sql, unknowns).is_some() {
                continue;
            }
            return Err(format!(
                "Unregistered Unknown inference for column {} ({}) against {backend}:\n  \
                 smelt inferred: {smelt_type:?}\n  \
                 {backend} actual:  {actual_type:?}\n  \
                 generating expr: {expr_sql}\n  \
                 SQL: {sql}\n  \
                 (add a prop_helpers/known_unknowns.rs entry or fix inference)",
                i, actual.0
            ));
        }

        match compare_types(smelt_type, actual_type) {
            TypeMatch::Exact | TypeMatch::Compatible { .. } => {}
            TypeMatch::Mismatch => {
                if find_divergence(smelt_type, actual_type, backend, divergences).is_none() {
                    return Err(format!(
                        "Type mismatch for column {} ({}) against {backend}:\n  \
                         smelt inferred: {smelt_type:?}\n  \
                         {backend} actual:  {actual_type:?}\n  \
                         SQL: {sql}",
                        i, actual.0
                    ));
                }
            }
        }
    }
    Ok(OracleCheckOutcome {
        columns_compared,
        query_refused: false,
    })
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
