//! Runtime-seam coverage for `docs/specs/incremental_models.md` §"Window
//! independence and self-referential models": an `Ordered` (convergent
//! self-edge) model composes with the derived output window exactly like a
//! `WindowIndependent` one, EXCEPT that the self-edge's own bounding relation
//! is never read as a partition-column skew anchor — only a genuine Form B
//! relation anchored on a *non-self* source contributes skew. (The pure-walk
//! coverage of the per-scope exclusion itself lives in
//! `crates/smelt-logical/tests/skew_self_exclusion.rs`; these tests exercise
//! the composition through `compute_incremental_windows_ordered`, the seam
//! the execute loop consumes.)
//!
//! `test_ordered_self_edge_alone_is_never_a_skew_anchor` is the false-positive
//! guard: `RUNNING_BALANCE_SQL`'s self-join condition
//! (`bal.d >= t.d - INTERVAL '1 day' AND bal.d < t.d`) reads, to a naive
//! text-level anchor scan, identically to a genuine skew declaration, because
//! the self-referenced table's own column shares the model's
//! `partition_column` name. `test_ordered_genuine_form_b_relation_still_derives_skew`
//! is the companion positive case: the same self-edge shape, but with an
//! additional Form B relation anchored on the *driving source's* own column,
//! must still derive and apply real skew.

use smelt_core::config::TimeseriesConfig;
use smelt_core::{Granularity, PartitionGrainConfig, PartitionGrainSafetyOverrides};
use smelt_logical::analysis::source_bounds::{Seconds, Skew};
use smelt_runtime::windowing::compute_incremental_windows_ordered;
use smelt_runtime::TimeRange;
use std::collections::HashMap;

fn make_ts(event_col: &str, partition_col: &str, granularity: Granularity) -> TimeseriesConfig {
    TimeseriesConfig {
        event_time_column: event_col.to_string(),
        partition_column: partition_col.to_string(),
        granularity,
        week_start: None,
        assert_monotonic: false,
    }
}

fn make_inc() -> PartitionGrainConfig {
    PartitionGrainConfig {
        unique_key: vec![],
        nondeterministic_columns_retired: (),
        safety_overrides: PartitionGrainSafetyOverrides::default(),
    }
}

fn make_range(start: &str, end: &str) -> TimeRange {
    TimeRange {
        start: start.to_string(),
        end: end.to_string(),
    }
}

fn one_source_dep(source: &str, partition_col: &str) -> HashMap<String, (Vec<String>, String)> {
    let mut m = HashMap::new();
    m.insert(
        source.to_string(),
        (
            source.split('.').map(String::from).collect(),
            partition_col.to_string(),
        ),
    );
    m
}

/// Identity-partition self-referential running-balance shape (the same SQL
/// `window_independence`'s own `backward_bounded_self_edge_is_ordered` unit
/// test and `windowing_parity.rs`'s BL7 tests use) — the self-join's ONLY
/// bounding relation is its own column, which happens to share the model's
/// `partition_column` name (`d`). No genuine skew exists anywhere in this SQL.
const RUNNING_BALANCE_SQL: &str = "SELECT bal.d AS d, bal.balance + t.amount AS balance \
     FROM smelt.marts.running_balance bal \
     JOIN smelt.silver.transactions t ON bal.acct_id = t.acct_id \
     WHERE bal.d >= t.d - INTERVAL '1 day' AND bal.d < t.d";

/// Same self-edge shape as `RUNNING_BALANCE_SQL` (a backward-bounded self-join
/// on the model's own partition column), PLUS a genuine Form B relation in the
/// WHERE clause anchored on the *driving source's own* column (`e.session_start_date`,
/// not the self-referenced table's `bal.session_start_date`) — a real 1-day/1-day
/// skew declaration, distinct from the self-edge's own bound.
const SESSION_SKEW_SQL: &str = "SELECT \
     e.session_start_date AS session_start_date, \
     bal.total + e.amt AS total \
     FROM smelt.sources.events e \
     LEFT JOIN smelt.sessions_like bal \
       ON bal.session_start_date >= e.session_start_date - INTERVAL '1 day' \
      AND bal.session_start_date < e.session_start_date \
     WHERE e.event_date BETWEEN e.session_start_date - INTERVAL '1 day' \
       AND e.session_start_date + INTERVAL '1 day'";

#[test]
fn test_ordered_self_edge_alone_is_never_a_skew_anchor() {
    let ts = make_ts("d", "d", Granularity::Day);
    let inc = make_inc();
    let range = make_range("2026-01-01", "2026-01-06"); // 5 days
    let deps = one_source_dep("silver.transactions", "d");
    let refs = vec![
        "marts.running_balance".to_string(),
        "silver.transactions".to_string(),
    ];

    let windows = compute_incremental_windows_ordered(
        "marts.running_balance",
        &refs,
        &ts,
        &inc,
        RUNNING_BALANCE_SQL,
        &deps,
        0,
        &range,
        None,
        false,
    )
    .expect("a convergent self-edge must build, not refuse");

    assert_eq!(
        windows.skew,
        Skew::ZERO,
        "the self-edge's own bounding relation must never be read as a \
         partition-column skew anchor, even though its column shares the \
         model's own partition_column name"
    );

    // No rebase: the batches must cover exactly the requested range, in
    // strictly sequential single-partition order (the pre-existing Ordered
    // forcing, unaffected by this phase).
    assert_eq!(windows.batches.len(), 5);
    assert_eq!(windows.batches[0].partition_start.to_string(), "2026-01-01");
    assert_eq!(
        windows.batches.last().unwrap().partition_end.to_string(),
        "2026-01-06"
    );
}

#[test]
fn test_ordered_genuine_form_b_relation_still_derives_skew() {
    let ts = make_ts("event_date", "session_start_date", Granularity::Day);
    let inc = make_inc();
    let range = make_range("2026-01-02", "2026-01-05"); // 3 days
    let deps = one_source_dep("sources.events", "session_start_date");
    let refs = vec!["sessions_like".to_string(), "sources.events".to_string()];

    let windows = compute_incremental_windows_ordered(
        "sessions_like",
        &refs,
        &ts,
        &inc,
        SESSION_SKEW_SQL,
        &deps,
        0,
        &range,
        None,
        false,
    )
    .expect("a convergent self-edge must build, not refuse");

    assert_eq!(
        windows.skew,
        Skew {
            before: Seconds::days(1),
            after: Seconds::days(1),
        },
        "the genuine Form B relation anchored on the driving source's own \
         column must still be derived, composing with the Ordered driver"
    );

    // The requested [2026-01-02, 2026-01-05) window must be rebased outward
    // by the derived skew to [2026-01-01, 2026-01-06) — the same Form B
    // write-rebase a window-independent model gets — while still building
    // strictly sequential single-partition batches (Ordered's own forcing).
    assert_eq!(windows.batches.len(), 5, "rebased range must span 5 days");
    assert_eq!(
        windows.batches[0].partition_start.to_string(),
        "2026-01-01",
        "rebase must reach one day earlier than the requested start"
    );
    assert_eq!(
        windows.batches.last().unwrap().partition_end.to_string(),
        "2026-01-06",
        "rebase must reach one day later than the requested end"
    );
    for batch in &windows.batches {
        let span_days = (batch.partition_end - batch.partition_start).num_days();
        assert_eq!(
            span_days, 1,
            "Ordered must still force single-partition batches over the rebased range"
        );
    }
}

/// A nested derived table reuses the self-edge's alias text (`bal`) for a
/// DIFFERENT source carrying the model's genuine Form B relation. The
/// exclusion is resolved per scope by the shared walk, so the inner scope's
/// relation must still widen the batches — a cross-scope alias accumulation
/// would drop it, under-widening the write window and stranding the
/// skew-reached partitions stale.
const SELF_ALIAS_REUSED_SQL: &str = "SELECT agg.d AS d, agg.total + bal.balance AS balance \
     FROM ( \
         SELECT bal.d AS d, SUM(bal.amt) AS total \
         FROM smelt.sources.ledger bal \
         WHERE bal.event_date BETWEEN bal.d - INTERVAL '1 day' \
             AND bal.d + INTERVAL '1 day' \
         GROUP BY bal.d \
     ) agg \
     LEFT JOIN smelt.marts.running_balance bal \
       ON bal.d >= agg.d - INTERVAL '3 days' AND bal.d < agg.d";

#[test]
fn test_ordered_alias_reuse_in_subquery_keeps_the_genuine_relation() {
    let ts = make_ts("event_date", "d", Granularity::Day);
    let inc = make_inc();
    let range = make_range("2026-01-03", "2026-01-04"); // 1 day
    let deps = one_source_dep("sources.ledger", "d");
    let refs = vec![
        "marts.running_balance".to_string(),
        "sources.ledger".to_string(),
    ];

    let windows = compute_incremental_windows_ordered(
        "marts.running_balance",
        &refs,
        &ts,
        &inc,
        SELF_ALIAS_REUSED_SQL,
        &deps,
        0,
        &range,
        None,
        false,
    )
    .expect("a convergent self-edge must build, not refuse");

    assert_eq!(
        windows.skew,
        Skew {
            before: Seconds::days(1),
            after: Seconds::days(1),
        },
        "the inner scope's genuine relation (same alias text as the self \
         edge) must survive the per-scope exclusion — dropping it would \
         under-widen the write window"
    );
    // The single requested day rebases outward to [D-1, D+1]: 3 sequential
    // single-partition batches.
    assert_eq!(windows.batches.len(), 3);
    assert_eq!(windows.batches[0].partition_start.to_string(), "2026-01-02");
    assert_eq!(
        windows.batches.last().unwrap().partition_end.to_string(),
        "2026-01-05"
    );
}
