//! `EXPERIMENTAL(property-discovery): disposable`
//!
//! Cell `P0-6` (`docs/research/20260705-property-discovery-loop.md` §2.2;
//! `docs/plans/20260705-property-discovery-loop.md` phase D).
//!
//! Link-B: classification-diagnostic scaffold. Reports smelt's analyzer facts
//! for a fixed model construct (`source_bounds::derive_model_bounds`'s reach,
//! `discriminants::combiner_discriminants`'s combiner facts) and independently
//! cross-checks the reach fact against a DuckDB **clamp-probe**: restrict the
//! source *read* to a candidate margin, apply the model's own filter on top,
//! and find the margin at which the result stops changing. This is the
//! ground-truth reach the model actually needs — compared against the
//! analyzer's derived answer.
//!
//! **Diagnostic only** (design §2.2) — it is never itself a green gate; Link C
//! (the execution gate, `smelt-cli`'s `property_discovery` test target)
//! decides HOLDS/REFUTED. This scaffold localizes *why*: a clamp-probe
//! mismatch would mean the analyzer's reach for this construct is
//! over-conservative or unsound before Link C ever runs.

use proptest::prelude::*;

use smelt_logical::analysis::discriminants::{combiner_discriminants, Discriminants};
use smelt_logical::analysis::source_bounds::{
    derive_model_bounds, BoundContext, BoundResult, Seconds,
};
use smelt_types::SqlFunction;

use crate::prop_helpers::duckdb_oracle::DuckDbOracle;

const PARTITION_DATE: &str = "2024-06-10";

/// The fixed model construct this cell classifies: an additive `SUM`
/// aggregation over a source read with a Form-B one-day lookback — the exact
/// shape of `source_bounds.rs::test_explicit_between_filter`. This is
/// `additive agg × append-only × fold-delta`'s Link-B facts (cell `G-01`'s
/// construct), reused here as the fixed subject for the classification
/// scaffold rather than duplicating a second bespoke model.
const MODEL_SQL: &str = "SELECT SUM(s.payload) AS total FROM sessions s \
     WHERE s.event_date BETWEEN m.partition_date - INTERVAL '1 day' AND m.partition_date";

/// smelt's own analyzer facts for [`MODEL_SQL`]'s source `silver.sessions`:
/// the combiner discriminants (`SUM` is an additive, non-idempotent monoid,
/// Link 0) and the derived reach (`source_bounds`'s Form-B walk).
fn analyzer_facts() -> (Discriminants, BoundResult) {
    let discriminants = combiner_discriminants(SqlFunction::Sum, false);
    let ctx = BoundContext::new().with_source("silver.sessions", "event_date");
    let bound = derive_model_bounds(MODEL_SQL, &ctx)
        .remove("silver.sessions")
        .expect("a Form-B filter over a declared timeseries source must derive a bound");
    (discriminants, bound)
}

fn setup_sessions(oracle: &DuckDbOracle, rows: &[(i64, i64)]) {
    oracle
        .execute_ddl("CREATE TABLE sessions (row_id BIGINT, event_date DATE, payload BIGINT)")
        .expect("create sessions table");
    for (i, (offset_days, payload)) in rows.iter().enumerate() {
        oracle
            .execute_ddl(&format!(
                "INSERT INTO sessions VALUES ({i}, DATE '{PARTITION_DATE}' + INTERVAL '{offset_days} day', {payload})"
            ))
            .expect("insert session row");
    }
}

/// The model's own true output: `SUM(payload)` over exactly the rows
/// [`MODEL_SQL`]'s Form-B filter admits (`[partition_date - 1 day,
/// partition_date]`).
fn true_sum(oracle: &DuckDbOracle) -> Option<i64> {
    query_sum(
        oracle,
        &format!(
            "SELECT SUM(payload) FROM sessions \
             WHERE event_date BETWEEN DATE '{PARTITION_DATE}' - INTERVAL '1 day' \
             AND DATE '{PARTITION_DATE}'"
        ),
    )
}

/// The **clamp-probe**: restrict the *read* to `margin_days` back from the
/// partition date (simulating a scan bounded to that reach), then apply the
/// model's own Form-B filter on top of that restricted read. If
/// `margin_days` is at least the true reach, restricting the read has no
/// effect (the model's own filter is already narrower-or-equal); if it is
/// less, rows the model wants are silently absent from the scan — exactly
/// the shape of an under-derived bound (`SC-1`'s hazard, at the ground-truth
/// level rather than smelt's).
fn clamped_sum(oracle: &DuckDbOracle, margin_days: i64) -> Option<i64> {
    query_sum(
        oracle,
        &format!(
            "SELECT SUM(payload) FROM (\
                 SELECT * FROM sessions \
                 WHERE event_date >= DATE '{PARTITION_DATE}' - INTERVAL '{margin_days} day'\
             ) restricted \
             WHERE event_date BETWEEN DATE '{PARTITION_DATE}' - INTERVAL '1 day' \
             AND DATE '{PARTITION_DATE}'"
        ),
    )
}

/// `DuckDbOracle::execute_query` renders each cell via `duckdb::types::Value`'s
/// `Debug` impl (e.g. `"HugeInt(42)"`, `"Null"`) rather than a bare value —
/// unwrap that wrapper for the single `SUM(...)` column this scaffold reads.
fn query_sum(oracle: &DuckDbOracle, sql: &str) -> Option<i64> {
    let rows = oracle.execute_query(sql).expect("query must succeed");
    let cell = rows.first()?.first()?;
    if cell == "Null" {
        return None;
    }
    let inner = cell
        .split_once('(')
        .map(|(_, rest)| rest.trim_end_matches(')'))
        .unwrap_or(cell.as_str());
    Some(
        inner
            .parse()
            .unwrap_or_else(|e| panic!("SUM column {cell:?} must parse as i64: {e}")),
    )
}

/// Rows as `(offset_days relative to partition_date, payload)`, spanning both
/// sides of the 1-day lookback boundary and beyond it.
fn arb_rows() -> impl Strategy<Value = Vec<(i64, i64)>> {
    proptest::collection::vec((-3_i64..=2, -50_i64..50), 0..8)
}

proptest! {
    /// Acceptance (design plan, phase D): the clamp-probe at the analyzer's
    /// derived margin (1 day, from `derive_model_bounds`) always agrees with
    /// the model's own true output — the derived reach is *sufficient*
    /// (never under-derives and silently drops a row the model wants).
    #[test]
    fn clamp_probe_at_derived_margin_matches_true_output(rows in arb_rows()) {
        let (_discriminants, bound) = analyzer_facts();
        let BoundResult::Bounded { before, after, .. } = bound else {
            panic!("expected a Bounded reach for this Form-B model, got {bound:?}");
        };
        prop_assert_eq!(after, Seconds::ZERO);
        let margin_days = (before.0 / 86_400) as i64;
        prop_assert_eq!(margin_days, 1, "sanity: this model's Form-B filter is a 1-day lookback");

        let oracle = DuckDbOracle::new();
        setup_sessions(&oracle, &rows);

        prop_assert_eq!(
            clamped_sum(&oracle, margin_days),
            true_sum(&oracle),
            "clamp-probe at the analyzer's derived margin ({}d) diverged from the \
             model's true output over rows {:?} — derive_model_bounds under-derived the reach \
             (unsound: a narrower scan than the model actually needs)",
            margin_days,
            rows
        );
    }
}

/// Deterministic witness (in the style of `maintenance_link_a.rs`'s
/// MIN-retraction witness — a targeted case, not a proptest search): a
/// margin of 0 days (one less than the derived bound) is INSUFFICIENT — a
/// row exactly at `partition_date - 1 day` is inside the model's own filter
/// but excluded by a 0-day-margin scan. This confirms 1 day is not just
/// *sufficient* (the proptest above) but *necessary* — the clamp-probe's
/// independently discovered ground-truth reach matches
/// `derive_model_bounds`'s answer exactly (sound **and** tight), not merely
/// conservatively safe.
#[test]
fn clamp_probe_at_one_less_than_derived_margin_diverges_on_boundary_row() {
    let oracle = DuckDbOracle::new();
    setup_sessions(&oracle, &[(-1, 42)]); // exactly at partition_date - 1 day

    let true_output = true_sum(&oracle);
    let clamped_at_zero_margin = clamped_sum(&oracle, 0);

    assert_eq!(
        true_output,
        Some(42),
        "the boundary row must be inside the model's own filter"
    );
    assert_ne!(
        clamped_at_zero_margin, true_output,
        "a 0-day-margin scan must miss the boundary row the 1-day derived bound correctly \
         reaches — this divergence IS the tightness witness, not a test bug"
    );
}
