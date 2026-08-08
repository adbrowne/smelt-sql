//! Phase 1 (`docs/plans/20260809-keyed-frontier.md`) — order-monotone
//! overwrite family (`MAX_BY`/`MIN_BY`) classification.
//!
//! See `docs/specs/incremental_models.md` §"The column-family catalogue" and
//! §"Ordering ties" for the normative spec these tests pin.

use std::collections::HashMap;

use smelt_core::config::{Granularity, TimeseriesConfig};
use smelt_logical::{
    classify_cumulative, CrossPartitionCombiner, KeyedDiagnostic, SourceTimeseriesMap,
};

fn ts(partition_col: &str) -> TimeseriesConfig {
    TimeseriesConfig {
        event_time_column: partition_col.to_string(),
        partition_column: partition_col.to_string(),
        granularity: Granularity::Day,
        week_start: None,
        assert_monotonic: false,
    }
}

fn events_source_map() -> SourceTimeseriesMap {
    let mut m = HashMap::new();
    m.insert("smelt.silver.events_parsed".to_string(), ts("event_date"));
    m
}

/// A `MAX_BY(value, ordering)` projection classifies as the order-monotone
/// overwrite family — no `KeyedUnknownCombiner` — as long as the ordering
/// expression is also projected via its own running `MAX(...)` column (the
/// classifier's storage decision: `rules/cumulative.rs`
/// `classify_order_monotone_column`'s doc comment).
#[test]
fn max_by_classifies_as_order_monotone_overwrite() {
    let sql = r#"SELECT
    device_id,
    MAX_BY(status, updated_at) AS status,
    MAX(updated_at) AS updated_at
FROM smelt.silver.events_parsed
GROUP BY device_id"#;
    let refs = vec!["smelt.silver.events_parsed".to_string()];
    let classification =
        classify_cumulative(sql, &refs, &events_source_map(), false).expect("must classify");

    assert_eq!(classification.unique_key, vec!["device_id"]);
    let status_col = classification
        .aggregator_columns
        .iter()
        .find(|c| c.output_name == "status")
        .expect("status column present");
    assert_eq!(status_col.per_partition_agg, "MAX_BY");
    match &status_col.cross_partition_combiner {
        CrossPartitionCombiner::OrderMonotone { ordering_column } => {
            assert_eq!(ordering_column, "updated_at");
        }
        other => panic!("expected OrderMonotone combiner, got {other:?}"),
    }

    // The companion tracking column is an ordinary extremal-fold column.
    let ord_col = classification
        .aggregator_columns
        .iter()
        .find(|c| c.output_name == "updated_at")
        .expect("updated_at tracking column present");
    assert_eq!(ord_col.per_partition_agg, "MAX");
    assert_eq!(
        ord_col.cross_partition_combiner,
        CrossPartitionCombiner::Max
    );
}

/// `MIN_BY` is the mirror family, tracked by a companion `MIN(...)` column.
#[test]
fn min_by_classifies_as_order_monotone_overwrite() {
    let sql = r#"SELECT
    device_id,
    MIN_BY(status, updated_at) AS status,
    MIN(updated_at) AS updated_at
FROM smelt.silver.events_parsed
GROUP BY device_id"#;
    let refs = vec!["smelt.silver.events_parsed".to_string()];
    let classification =
        classify_cumulative(sql, &refs, &events_source_map(), false).expect("must classify");

    let status_col = classification
        .aggregator_columns
        .iter()
        .find(|c| c.output_name == "status")
        .expect("status column present");
    match &status_col.cross_partition_combiner {
        CrossPartitionCombiner::OrderMonotone { ordering_column } => {
            assert_eq!(ordering_column, "updated_at");
        }
        other => panic!("expected OrderMonotone combiner, got {other:?}"),
    }
}

/// Mixed-family projections fold column-wise: `SUM` + `MAX_BY` (with its
/// companion tracking column) + `MIN` in one model, three families, no
/// whole-model refusal.
#[test]
fn mixed_family_projection_classifies_columnwise() {
    let sql = r#"SELECT
    device_id,
    SUM(amount) AS total_amount,
    MAX_BY(status, updated_at) AS status,
    MAX(updated_at) AS updated_at,
    MIN(first_seen) AS first_seen
FROM smelt.silver.events_parsed
GROUP BY device_id"#;
    let refs = vec!["smelt.silver.events_parsed".to_string()];
    let classification =
        classify_cumulative(sql, &refs, &events_source_map(), false).expect("must classify");

    assert_eq!(classification.aggregator_columns.len(), 4);
    let by_name = |name: &str| {
        classification
            .aggregator_columns
            .iter()
            .find(|c| c.output_name == name)
            .unwrap_or_else(|| panic!("missing column {name}"))
    };
    assert_eq!(
        by_name("total_amount").cross_partition_combiner,
        CrossPartitionCombiner::Sum
    );
    assert!(matches!(
        by_name("status").cross_partition_combiner,
        CrossPartitionCombiner::OrderMonotone { .. }
    ));
    assert_eq!(
        by_name("updated_at").cross_partition_combiner,
        CrossPartitionCombiner::Max
    );
    assert_eq!(
        by_name("first_seen").cross_partition_combiner,
        CrossPartitionCombiner::Min
    );
}

/// A bare non-aggregate, non-key column is still `KeyedUnknownCombiner` —
/// plain overwrite is the snapshot-shape family (Phase 3), not admitted
/// window-forward.
#[test]
fn bare_nonkey_projection_still_unknown() {
    let sql = r#"SELECT
    device_id,
    status
FROM smelt.silver.events_parsed
GROUP BY device_id"#;
    let refs = vec!["smelt.silver.events_parsed".to_string()];
    let err = classify_cumulative(sql, &refs, &events_source_map(), false).unwrap_err();
    assert!(
        err.iter()
            .any(|d| matches!(d, KeyedDiagnostic::KeyedUnknownCombiner { .. })),
        "diagnostics: {:?}",
        err
    );
}

/// `MAX_BY` whose ordering expression is NOT also projected as a running
/// `MAX(...)` column refuses `KeyedUnknownCombiner` — the classifier stores
/// no hidden ordering state (Phase 1's storage decision).
#[test]
fn max_by_without_tracking_column_refuses_unknown_combiner() {
    let sql = r#"SELECT
    device_id,
    MAX_BY(status, updated_at) AS status
FROM smelt.silver.events_parsed
GROUP BY device_id"#;
    let refs = vec!["smelt.silver.events_parsed".to_string()];
    let err = classify_cumulative(sql, &refs, &events_source_map(), false).unwrap_err();
    assert!(
        err.iter().any(|d| matches!(
            d,
            KeyedDiagnostic::KeyedUnknownCombiner { offending, .. }
                if offending.contains("updated_at")
        )),
        "diagnostics: {:?}",
        err
    );
}

/// A composite expression wrapping `MAX_BY` (e.g. `MAX_BY(...) + 1`) is
/// refused, mirroring the direct-monoid families' composite-expression rule.
#[test]
fn composite_max_by_expression_refused() {
    let sql = r#"SELECT
    device_id,
    MAX_BY(status, updated_at) + 1 AS status,
    MAX(updated_at) AS updated_at
FROM smelt.silver.events_parsed
GROUP BY device_id"#;
    let refs = vec!["smelt.silver.events_parsed".to_string()];
    let err = classify_cumulative(sql, &refs, &events_source_map(), false).unwrap_err();
    assert!(
        err.iter().any(|d| matches!(
            d,
            KeyedDiagnostic::KeyedUnknownCombiner { offending, .. }
                if offending.contains("composite") || offending.contains("expression")
        )),
        "diagnostics: {:?}",
        err
    );
}

/// A `MAX_BY` column under the (still unbuilt) snapshot-reconcile posture —
/// zero clocked driving sources — refuses `KeyedSnapshotPostureUnsupported`,
/// same as any other keyed model with no timeseries-tagged source. (The
/// admission-matrix ✗ direction proper — refusing `MAX_BY` under
/// snapshot-reconcile with an "observer semantics" reason — is Phase 3's
/// concern once the snapshot-reconcile run shape exists; today every
/// zero-clocked-source model refuses at this earlier gate regardless of
/// column family.)
#[test]
fn max_by_under_zero_clocked_sources_refuses_snapshot_posture_unsupported() {
    let sql = r#"SELECT
    device_id,
    MAX_BY(status, updated_at) AS status,
    MAX(updated_at) AS updated_at
FROM smelt.silver.lookup_table
GROUP BY device_id"#;
    let refs = vec!["smelt.silver.lookup_table".to_string()];
    let err = classify_cumulative(sql, &refs, &HashMap::new(), false).unwrap_err();
    assert!(
        err.iter()
            .any(|d| matches!(d, KeyedDiagnostic::KeyedSnapshotPostureUnsupported)),
        "diagnostics: {:?}",
        err
    );
}

/// Cross-partition combiner rendering: the `OrderMonotone` merge compares
/// ordering values with strict `>` (incumbent wins ties, §"Ordering ties").
#[test]
fn order_monotone_combiner_renders_incumbent_wins_comparison() {
    let combiner = CrossPartitionCombiner::OrderMonotone {
        ordering_column: "updated_at".to_string(),
    };
    let rendered = combiner.render("target.status", "delta.status");
    assert_eq!(
        rendered,
        "CASE WHEN delta.updated_at > target.updated_at THEN delta.status ELSE target.status END"
    );
}
