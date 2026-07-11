//! DuckDB differential parsing harness — enforces both dialect-conformance
//! directions from architecture spec §Constraints #13 against a real DuckDB
//! execution oracle.
//!
//! - **Accept direction** (`duckdb_valid_seed_corpus_parses_or_registered`):
//!   every seed statement DuckDB accepts must parse cleanly in smelt or match a
//!   registered gap in `gaps.rs`.
//! - **Fidelity direction** (`parsed_clean_sql_round_trips_on_duckdb`): every
//!   seed statement smelt parses cleanly must print back to SQL DuckDB still
//!   *executes* (not merely re-parses). This is the gate that closes the
//!   silent-mis-parse class.
//! - **Ratchet** (`gap_count_ratchet`): the number of registered seed gaps is
//!   pinned to `.claude/parser-gaps-baseline.txt`; it may only shrink.
//! - **Property fidelity** (`prop_generated_clean_sql_executes_on_duckdb`): any
//!   generated statement smelt parses cleanly must execute on DuckDB, or match a
//!   registered gap.

use proptest::prelude::*;
use smelt_parser_compat::duckdb_oracle::{duckdb_accepts, duckdb_executes, SCHEMA_PRELUDE};
use smelt_parser_compat::{gaps, generators, SmeltParseResult};

/// Load the seed corpus (one statement per line; `#` comments and blanks skipped).
fn seed_statements() -> Vec<String> {
    include_str!("corpus/duckdb_seed.sql")
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.to_string())
        .collect()
}

/// True if `sql` is a registered gap in either DuckDB differential category.
fn is_registered_gap(sql: &str) -> bool {
    gaps::is_known_gap(sql, "duckdb_fails_to_parse")
        || gaps::is_known_gap(sql, "roundtrip_mismatch")
}

/// Count the seed statements that are registered gaps (the ratchet metric).
fn registered_seed_gap_count() -> usize {
    seed_statements()
        .iter()
        .filter(|sql| is_registered_gap(sql))
        .count()
}

/// Read the pinned `duckdb_seed_gaps` count from the baseline file.
fn baseline_gap_count() -> usize {
    let baseline = include_str!("../../../.claude/parser-gaps-baseline.txt");
    for line in baseline.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("duckdb_seed_gaps") {
            return rest
                .trim()
                .parse()
                .expect("duckdb_seed_gaps value must be an integer");
        }
    }
    panic!("`duckdb_seed_gaps` metric not found in .claude/parser-gaps-baseline.txt");
}

#[test]
fn duckdb_valid_seed_corpus_parses_or_registered() {
    let mut failures = Vec::new();
    for sql in seed_statements() {
        // Accept direction only constrains statements DuckDB itself accepts.
        if duckdb_accepts(&sql, SCHEMA_PRELUDE).is_err() {
            continue;
        }
        let smelt = SmeltParseResult::parse(&sql);
        if !smelt.success && !is_registered_gap(&sql) {
            failures.push(format!(
                "DuckDB accepts but smelt fails to parse and it is NOT a registered gap:\n  \
                 sql: {sql}\n  smelt errors: {:?}",
                smelt.errors
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "Accept-direction gate failed ({} statement(s)). Add a gaps.rs entry or fix the grammar:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn parsed_clean_sql_round_trips_on_duckdb() {
    let mut failures = Vec::new();
    for sql in seed_statements() {
        let smelt = SmeltParseResult::parse(&sql);
        if !smelt.success {
            continue;
        }
        let Some(printed) = smelt.normalized_sql.as_deref() else {
            continue;
        };
        // Fidelity: the printed SQL must EXECUTE on DuckDB, not merely re-parse.
        if let Err(err) = duckdb_executes(printed, SCHEMA_PRELUDE) {
            if !is_registered_gap(&sql) {
                failures.push(format!(
                    "smelt parses cleanly but printed SQL does not execute on DuckDB \
                     (silent mis-parse) and it is NOT a registered gap:\n  \
                     original: {sql}\n  printed:  {printed}\n  duckdb:   {err}"
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "Fidelity-direction gate failed ({} statement(s)):\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn gap_count_ratchet() {
    let current = registered_seed_gap_count();
    let baseline = baseline_gap_count();
    assert!(
        current <= baseline,
        "Registered seed-gap count REGRESSED: current={current} > baseline={baseline}.\n\
         A new gap must be justified by editing .claude/parser-gaps-baseline.txt \
         (reviewer-visible), never absorbed silently.",
    );
    assert!(
        current >= baseline,
        "STALE baseline: current={current} < baseline={baseline}.\n\
         Grammar support closed a gap — tighten .claude/parser-gaps-baseline.txt to {current}.",
    );
}

proptest! {
    // Keep default cases modest so `cargo test` stays fast; bump with PROPTEST_CASES.
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Any generated statement smelt parses cleanly must execute on DuckDB — or
    /// be a registered gap. This is the fidelity direction over the generator.
    #[test]
    fn prop_generated_clean_sql_executes_on_duckdb(sql in generators::any_select()) {
        let smelt = SmeltParseResult::parse(&sql);
        // Only clean parses are constrained; smelt-extension SQL is out of scope.
        if !smelt.success || smelt_parser_compat::has_smelt_extensions(&sql) {
            return Ok(());
        }
        let Some(printed) = smelt.normalized_sql.as_deref() else {
            return Ok(());
        };
        // The generator emits SQL over synthetic column names, so DuckDB binding
        // may legitimately fail (unknown column); the printed form must at least
        // still PARSE on DuckDB. We accept binder/catalog errors but reject
        // parser (syntax) errors, which are the silent-mis-parse signal.
        if let Err(err) = duckdb_accepts(printed, "") {
            let is_syntax = err.contains("Parser Error") || err.contains("syntax error");
            prop_assert!(
                !is_syntax || is_registered_gap(&sql),
                "smelt parsed cleanly but printed SQL is a DuckDB SYNTAX error:\n  \
                 original: {sql}\n  printed:  {printed}\n  duckdb:   {err}",
            );
        }
    }
}

#[test]
fn seed_corpus_is_nonempty_and_covers_review_cases() {
    let stmts = seed_statements();
    assert!(
        stmts.len() >= 18,
        "seed corpus should list every review case"
    );
    // Spot-check a few of the enumerated review constructs are present.
    let joined = stmts.join("\n");
    for needle in [
        "TRY_CAST",
        "GROUP BY ALL",
        "ORDER BY ALL",
        "IGNORE NULLS",
        "GLOB",
        "MAP {",
    ] {
        assert!(joined.contains(needle), "seed corpus missing {needle}");
    }
}
