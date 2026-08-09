//! Phase 1 (`docs/plans/20260809-keyed-frontier.md`) — order-monotone
//! overwrite family (`MAX_BY`/`MIN_BY`) classification.
//!
//! See `docs/specs/incremental_models.md` §"The column-family catalogue" and
//! §"Ordering ties" for the normative spec these tests pin.

use std::collections::HashMap;

use smelt_core::config::{FunctionalDependency, Granularity, TimeseriesConfig};
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
        classify_cumulative(sql, &refs, &events_source_map(), false, &[]).expect("must classify");

    assert_eq!(classification.unique_key, vec!["device_id"]);
    let status_col = classification
        .aggregator_columns
        .iter()
        .find(|c| c.output_name == "status")
        .expect("status column present");
    assert_eq!(status_col.per_partition_agg, "MAX_BY");
    match &status_col.cross_partition_combiner {
        CrossPartitionCombiner::OrderMonotone {
            ordering_column,
            prefer_greater,
        } => {
            assert_eq!(ordering_column, "updated_at");
            let _ = prefer_greater;
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
        classify_cumulative(sql, &refs, &events_source_map(), false, &[]).expect("must classify");

    let status_col = classification
        .aggregator_columns
        .iter()
        .find(|c| c.output_name == "status")
        .expect("status column present");
    match &status_col.cross_partition_combiner {
        CrossPartitionCombiner::OrderMonotone {
            ordering_column,
            prefer_greater,
        } => {
            assert_eq!(ordering_column, "updated_at");
            let _ = prefer_greater;
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
        classify_cumulative(sql, &refs, &events_source_map(), false, &[]).expect("must classify");

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
    let err = classify_cumulative(sql, &refs, &events_source_map(), false, &[]).unwrap_err();
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
    let err = classify_cumulative(sql, &refs, &events_source_map(), false, &[]).unwrap_err();
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
    let err = classify_cumulative(sql, &refs, &events_source_map(), false, &[]).unwrap_err();
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

/// A refusal must never suggest a spelling that itself refuses. The
/// once-write family's admitted reduction (`COALESCE(MAX(<col>))`) requires a
/// bare column reference as the `MAX` argument
/// (`docs/specs/incremental_models.md` §"The column-family catalogue"), so a
/// composite projection like `a || b` must NOT be offered that spelling —
/// only the `MAX_BY`/`ANY_VALUE` fixes, which do accept an expression.
#[test]
fn composite_projection_refusal_does_not_suggest_a_refusing_once_write_spelling() {
    let sql = r#"SELECT
    device_id,
    status || reason AS combined
FROM smelt.silver.events_parsed
GROUP BY device_id"#;
    let refs = vec!["smelt.silver.events_parsed".to_string()];
    let err = classify_cumulative(sql, &refs, &events_source_map(), false, &[]).unwrap_err();
    let offending = err
        .iter()
        .find_map(|d| match d {
            KeyedDiagnostic::KeyedUnknownCombiner { offending, .. } => Some(offending.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("expected KeyedUnknownCombiner, got {err:?}"));
    assert!(
        !offending.contains("COALESCE"),
        "a composite projection must not be offered the once-write spelling, which \
         `classify_once_write` route 2 would itself refuse: {offending}"
    );
    assert!(
        offending.contains("MAX_BY") && offending.contains("ANY_VALUE"),
        "the remaining overwrite-family suggestions must still be offered: {offending}"
    );
}

/// A **bare** non-key column reference keeps the once-write suggestion — its
/// `COALESCE(MAX(<col>))` spelling is exactly what route 2 admits under a
/// declared functional dependency.
#[test]
fn bare_column_refusal_still_suggests_the_once_write_spelling() {
    let sql = r#"SELECT
    device_id,
    status
FROM smelt.silver.events_parsed
GROUP BY device_id"#;
    let refs = vec!["smelt.silver.events_parsed".to_string()];
    let err = classify_cumulative(sql, &refs, &events_source_map(), false, &[]).unwrap_err();
    let offending = err
        .iter()
        .find_map(|d| match d {
            KeyedDiagnostic::KeyedUnknownCombiner { offending, .. } => Some(offending.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("expected KeyedUnknownCombiner, got {err:?}"));
    assert!(
        offending.contains("COALESCE(MAX(status))"),
        "a bare column reference must still name the once-write reduction: {offending}"
    );
}

/// Zero clocked driving sources derives the snapshot-reconcile run shape
/// (Phase 3, `docs/plans/20260809-keyed-frontier.md`) instead of refusing
/// the whole model: a plain `ANY_VALUE(...)` projection (the plain-overwrite
/// family, `incremental_models.md` §"The column-family catalogue") admits
/// cleanly, and the classification records no clocked timeseries.
#[test]
fn zero_clocked_sources_derives_snapshot_reconcile() {
    let sql = r#"SELECT
    device_id,
    ANY_VALUE(status) AS status
FROM smelt.silver.lookup_table
GROUP BY device_id"#;
    let refs = vec!["smelt.silver.lookup_table".to_string()];
    let classification = classify_cumulative(sql, &refs, &HashMap::new(), false, &[])
        .expect("plain-overwrite must classify under snapshot-reconcile");

    assert!(
        classification.is_snapshot_reconcile(),
        "expected the snapshot-reconcile run shape (no clocked driving source): {:?}",
        classification
    );
    assert!(
        classification.driving_source.timeseries.is_none(),
        "snapshot-reconcile carries no clocked driving-source timeseries"
    );
    let status_col = classification
        .aggregator_columns
        .iter()
        .find(|c| c.output_name == "status")
        .expect("status column present");
    assert_eq!(status_col.per_partition_agg, "ANY_VALUE");
    assert_eq!(
        status_col.cross_partition_combiner,
        CrossPartitionCombiner::PlainOverwrite
    );
}

/// The admission matrix's snapshot-reconcile ✗ column: an additive-fold
/// (`SUM`) projection under zero clocked sources refuses
/// `KeyedSnapshotSourceUnsupportedColumn` naming the double-count reason —
/// the posture itself is supportable now, only the fold family is refused.
#[test]
fn additive_fold_refuses_snapshot_reconcile() {
    let sql = r#"SELECT
    device_id,
    SUM(amount) AS total_amount
FROM smelt.silver.lookup_table
GROUP BY device_id"#;
    let refs = vec!["smelt.silver.lookup_table".to_string()];
    let err = classify_cumulative(sql, &refs, &HashMap::new(), false, &[]).unwrap_err();
    assert!(
        !err.iter()
            .any(|d| matches!(d, KeyedDiagnostic::KeyedSnapshotPostureUnsupported)),
        "the posture itself (a single unclocked source) is supportable: {:?}",
        err
    );
    assert!(
        err.iter().any(|d| matches!(
            d,
            KeyedDiagnostic::KeyedSnapshotSourceUnsupportedColumn { family, reason, .. }
                if family == "additive fold" && reason.contains("double-count")
        )),
        "diagnostics: {:?}",
        err
    );
}

/// The admission matrix's snapshot-reconcile ✗ column, order-monotone
/// overwrite direction: `MAX_BY` under zero clocked sources refuses
/// `KeyedSnapshotSourceUnsupportedColumn` naming observer semantics — the
/// same posture that admits a bare `ANY_VALUE` column ([`zero_clocked_
/// sources_derives_snapshot_reconcile`]) still refuses a fold-family column.
#[test]
fn order_monotone_refuses_snapshot_reconcile() {
    let sql = r#"SELECT
    device_id,
    MAX_BY(status, updated_at) AS status,
    MAX(updated_at) AS updated_at
FROM smelt.silver.lookup_table
GROUP BY device_id"#;
    let refs = vec!["smelt.silver.lookup_table".to_string()];
    let err = classify_cumulative(sql, &refs, &HashMap::new(), false, &[]).unwrap_err();
    assert!(
        !err.iter()
            .any(|d| matches!(d, KeyedDiagnostic::KeyedSnapshotPostureUnsupported)),
        "the posture itself (a single unclocked source) is supportable: {:?}",
        err
    );
    assert!(
        err.iter().any(|d| matches!(
            d,
            KeyedDiagnostic::KeyedSnapshotSourceUnsupportedColumn { family, reason, .. }
                if family == "order-monotone overwrite" && reason.contains("observer semantics")
        )),
        "diagnostics: {:?}",
        err
    );
}

/// The admission matrix's other direction: a plain (`ANY_VALUE`) column
/// under a CLOCKED driving source (window-forward) refuses
/// `KeyedUnknownCombiner`, and the message names `MAX_BY` as the fix.
#[test]
fn plain_overwrite_refuses_window_forward_with_max_by_hint() {
    let sql = r#"SELECT
    device_id,
    ANY_VALUE(status) AS status
FROM smelt.silver.events_parsed
GROUP BY device_id"#;
    let refs = vec!["smelt.silver.events_parsed".to_string()];
    let err = classify_cumulative(sql, &refs, &events_source_map(), false, &[]).unwrap_err();
    assert!(
        err.iter().any(|d| matches!(
            d,
            KeyedDiagnostic::KeyedUnknownCombiner { offending, .. }
                if offending.contains("MAX_BY")
        )),
        "expected the KeyedUnknownCombiner refusal to name MAX_BY as the fix: {:?}",
        err
    );
}

/// Cross-partition combiner rendering: the `OrderMonotone` merge compares
/// ordering values with strict `>` (incumbent wins ties, §"Ordering ties").
#[test]
fn order_monotone_combiner_renders_incumbent_wins_comparison() {
    let combiner = CrossPartitionCombiner::OrderMonotone {
        ordering_column: "updated_at".to_string(),
        prefer_greater: true,
    };
    let rendered = combiner.render("target.status", "delta.status");
    assert_eq!(
        rendered,
        "CASE WHEN delta.updated_at > target.updated_at THEN delta.status ELSE target.status END"
    );
}

// ---------------------------------------------------------------------
// Phase 4 (`docs/plans/20260809-keyed-frontier.md`): the once-write family
// (`COALESCE`), with the once-write provenance proof — key-derived, or a
// declared functional dependency consulted via
// `functional_dependency_verdict_over_vector` (`incremental_models.md`
// §"The column-family catalogue").
// ---------------------------------------------------------------------

/// A `COALESCE(MAX(<col>))` projection admits as the once-write family when
/// a declared `functional_dependencies: key -> col` entry proves the SOURCE
/// payload column `col` a per-key constant — the FD verdict is genuinely
/// consulted (closing `model_properties.md`'s "no consumer wired"
/// divergence).
#[test]
fn coalesce_with_declared_fd_classifies_once_write() {
    let sql = r#"SELECT
    device_id,
    COALESCE(MAX(signup_referrer)) AS first_referrer
FROM smelt.silver.events_parsed
GROUP BY device_id"#;
    let refs = vec!["smelt.silver.events_parsed".to_string()];
    let fds = vec![FunctionalDependency {
        key: vec!["device_id".to_string()],
        determines: "signup_referrer".to_string(),
    }];
    let classification = classify_cumulative(sql, &refs, &events_source_map(), false, &fds)
        .expect("declared FD must admit the once-write column");

    let col = classification
        .aggregator_columns
        .iter()
        .find(|c| c.output_name == "first_referrer")
        .expect("first_referrer column present");
    assert_eq!(col.per_partition_agg, "COALESCE");
    assert_eq!(
        col.cross_partition_combiner,
        CrossPartitionCombiner::OnceWrite
    );
}

/// The world-fact the once-write equivalence needs is `key -> <source
/// payload column>`, never `key -> <output alias>`: an aggregate over the
/// model's own `GROUP BY` key determines its own alias by construction, so
/// a declaration naming the alias asserts nothing. Such a declaration must
/// NOT admit.
#[test]
fn declared_fd_naming_the_output_alias_does_not_admit() {
    let sql = r#"SELECT
    device_id,
    COALESCE(MAX(signup_referrer), 'unknown') AS first_referrer
FROM smelt.silver.events_parsed
GROUP BY device_id"#;
    let refs = vec!["smelt.silver.events_parsed".to_string()];
    let fds = vec![FunctionalDependency {
        key: vec!["device_id".to_string()],
        // The projection's own output alias — vacuously determined by the
        // GROUP BY key, so it proves nothing about the payload's provenance.
        determines: "first_referrer".to_string(),
    }];
    let err = classify_cumulative(sql, &refs, &events_source_map(), false, &fds).unwrap_err();
    assert!(
        err.iter().any(|d| matches!(
            d,
            KeyedDiagnostic::KeyedOnceWriteUnproven { column, .. } if column == "signup_referrer"
        )),
        "diagnostics: {err:?}"
    );
}

/// A declared key that is a SUBSET of the model's `unique_key` is a
/// STRONGER statement than the full-key FD (it implies it), so it admits —
/// matching `functional_dependency_verdict_over_vector`'s own
/// `has_subset_key` treatment.
#[test]
fn declared_fd_over_a_subset_key_admits() {
    let sql = r#"SELECT
    device_id,
    user_id,
    COALESCE(MAX(signup_referrer)) AS first_referrer
FROM smelt.silver.events_parsed
GROUP BY device_id, user_id"#;
    let refs = vec!["smelt.silver.events_parsed".to_string()];
    let fds = vec![FunctionalDependency {
        key: vec!["device_id".to_string()],
        determines: "signup_referrer".to_string(),
    }];
    let classification = classify_cumulative(sql, &refs, &events_source_map(), false, &fds)
        .expect("a subset-key declaration implies the unique_key FD and must admit");
    let col = classification
        .aggregator_columns
        .iter()
        .find(|c| c.output_name == "first_referrer")
        .expect("first_referrer column present");
    assert_eq!(
        col.cross_partition_combiner,
        CrossPartitionCombiner::OnceWrite
    );
}

/// The widen-only law's second structural leg (`model_properties.md` SC-6):
/// an undiscriminated `UNION ALL` body is a structural disproof a
/// declaration cannot widen past — an FD holding in each branch need not
/// hold in the union.
#[test]
fn coalesce_with_set_op_barrier_refuses_despite_declared_fd() {
    let sql = r#"SELECT
    device_id,
    COALESCE(MAX(signup_referrer), 'unknown') AS first_referrer
FROM (
    SELECT device_id, signup_referrer FROM smelt.silver.events_parsed
    UNION ALL
    SELECT device_id, signup_referrer FROM smelt.silver.events_legacy
)
GROUP BY device_id"#;
    let refs = vec![
        "smelt.silver.events_parsed".to_string(),
        "smelt.silver.events_legacy".to_string(),
    ];
    let fds = vec![FunctionalDependency {
        key: vec!["device_id".to_string()],
        determines: "signup_referrer".to_string(),
    }];
    let err = classify_cumulative(sql, &refs, &events_source_map(), false, &fds).unwrap_err();
    assert!(
        err.iter().any(|d| matches!(
            d,
            KeyedDiagnostic::KeyedOnceWriteUnproven { reason, .. }
                if reason.contains("UNION ALL") || reason.contains("set operation")
        )),
        "diagnostics: {err:?}"
    );
}

/// Regression: a `COALESCE(...)` expression used as a null-safe composite
/// GROUP BY key is a KEY column, not a once-write column — the once-write
/// arm must not claim (and refuse) it.
#[test]
fn coalesce_group_by_key_is_a_key_not_once_write() {
    let sql = r#"SELECT
    COALESCE(device_id, 'n/a') AS device_key,
    SUM(amount) AS total
FROM smelt.silver.events_parsed
GROUP BY COALESCE(device_id, 'n/a')"#;
    let refs = vec!["smelt.silver.events_parsed".to_string()];
    let classification = classify_cumulative(sql, &refs, &events_source_map(), false, &[])
        .expect("a null-safe composite GROUP BY key must classify");
    assert_eq!(classification.unique_key, vec!["device_key"]);
    assert!(
        !classification
            .aggregator_columns
            .iter()
            .any(|c| c.output_name == "device_key"),
        "the GROUP BY key must not be claimed by the once-write family: {:?}",
        classification.aggregator_columns
    );
}

/// The widen-only law: a declared FD cannot override a *proven* fan-out —
/// when the coalesced column is sourced from a row-multiplying join, the
/// once-write column refuses (`KeyedOnceWriteUnproven`) regardless of the
/// declaration.
#[test]
fn coalesce_with_fan_out_join_refuses_despite_declared_fd() {
    let sql = r#"SELECT
    e.device_id,
    COALESCE(MAX(o.referrer), 'unknown') AS signup_referrer
FROM smelt.silver.events_parsed e
JOIN smelt.silver.orders o ON o.device_id = e.device_id
GROUP BY e.device_id"#;
    let refs = vec![
        "smelt.silver.events_parsed".to_string(),
        "smelt.silver.orders".to_string(),
    ];
    let fds = vec![FunctionalDependency {
        key: vec!["device_id".to_string()],
        determines: "signup_referrer".to_string(),
    }];
    let err = classify_cumulative(sql, &refs, &events_source_map(), false, &fds).unwrap_err();
    assert!(
        err.iter().any(|d| matches!(
            d,
            KeyedDiagnostic::KeyedOnceWriteUnproven { column, .. } if column == "referrer"
        )),
        "diagnostics: {err:?}"
    );
}

/// The declared FD's `key` names real SOURCE columns, not the projection's
/// output aliases. `SELECT user_id AS device_id ... GROUP BY user_id`
/// groups by `user_id`; a declaration `key: [device_id]` names a different
/// column than the model actually groups on and so proves nothing about the
/// payload's per-key constancy. It must NOT admit.
#[test]
fn declared_fd_key_naming_an_alias_of_a_different_source_column_does_not_admit() {
    let sql = r#"SELECT
    user_id AS device_id,
    COALESCE(MAX(signup_referrer)) AS first_referrer
FROM smelt.silver.events_parsed
GROUP BY user_id"#;
    let refs = vec!["smelt.silver.events_parsed".to_string()];
    let fds = vec![FunctionalDependency {
        // The output alias of the grouping projection — the model groups by
        // `user_id`, so this declaration names a different column.
        key: vec!["device_id".to_string()],
        determines: "signup_referrer".to_string(),
    }];
    let err = classify_cumulative(sql, &refs, &events_source_map(), false, &fds).unwrap_err();
    assert!(
        err.iter().any(|d| matches!(
            d,
            KeyedDiagnostic::KeyedOnceWriteUnproven { column, .. } if column == "signup_referrer"
        )),
        "diagnostics: {err:?}"
    );

    // The same declaration naming the actual grouping SOURCE column admits.
    let source_fds = vec![FunctionalDependency {
        key: vec!["user_id".to_string()],
        determines: "signup_referrer".to_string(),
    }];
    let classification = classify_cumulative(sql, &refs, &events_source_map(), false, &source_fds)
        .expect("a declaration naming the grouping source column must admit");
    let col = classification
        .aggregator_columns
        .iter()
        .find(|c| c.output_name == "first_referrer")
        .expect("first_referrer column present");
    assert_eq!(
        col.cross_partition_combiner,
        CrossPartitionCombiner::OnceWrite
    );
}

/// A key-derived coalesced value (a bare reference to a `unique_key`
/// column) admits without any declared functional dependency.
#[test]
fn key_derived_once_write_needs_no_declaration() {
    let sql = r#"SELECT
    device_id,
    COALESCE(device_id, 'n/a') AS first_seen_device
FROM smelt.silver.events_parsed
GROUP BY device_id"#;
    let refs = vec!["smelt.silver.events_parsed".to_string()];
    let classification = classify_cumulative(sql, &refs, &events_source_map(), false, &[])
        .expect("a key-derived COALESCE column needs no FD declaration");

    let col = classification
        .aggregator_columns
        .iter()
        .find(|c| c.output_name == "first_seen_device")
        .expect("first_seen_device column present");
    assert_eq!(
        col.cross_partition_combiner,
        CrossPartitionCombiner::OnceWrite
    );
}

/// No declaration and no key-derived proof: `KeyedOnceWriteUnproven` names
/// the source payload column and the three fixes.
#[test]
fn unproven_once_write_refuses_with_three_fixes() {
    let sql = r#"SELECT
    device_id,
    COALESCE(MAX(signup_referrer)) AS first_referrer
FROM smelt.silver.events_parsed
GROUP BY device_id"#;
    let refs = vec!["smelt.silver.events_parsed".to_string()];
    let err = classify_cumulative(sql, &refs, &events_source_map(), false, &[]).unwrap_err();
    let diag = err
        .iter()
        .find(|d| matches!(d, KeyedDiagnostic::KeyedOnceWriteUnproven { .. }))
        .expect("expected KeyedOnceWriteUnproven");
    let KeyedDiagnostic::KeyedOnceWriteUnproven { column, .. } = diag else {
        unreachable!()
    };
    assert_eq!(column, "signup_referrer");
    let message = diag.to_string();
    assert!(
        message.contains("key-derived")
            && message.contains("functional dependency")
            && message.contains("remodelling"),
        "expected the three named fixes: {message}"
    );
}

/// A literal fallback (`COALESCE(MAX(val), -1)`) makes the projection
/// TOTAL, which breaks the equivalence invariant: a window in which every
/// row for a key carries `val IS NULL` writes the literal into the target,
/// and `COALESCE(target, delta)` then locks that default in forever — a
/// later window delivering a real value can never displace it, while a full
/// refresh would return the real value. The declared FD asserts "per-key
/// constant", never "never NULL", and this family is literally "first
/// NON-NULL", so intra-key NULLs are anticipated. Refuse.
#[test]
fn once_write_with_literal_fallback_refuses_despite_declared_fd() {
    let sql = r#"SELECT
    device_id,
    COALESCE(MAX(signup_referrer), 'unknown') AS first_referrer
FROM smelt.silver.events_parsed
GROUP BY device_id"#;
    let refs = vec!["smelt.silver.events_parsed".to_string()];
    let fds = vec![FunctionalDependency {
        key: vec!["device_id".to_string()],
        determines: "signup_referrer".to_string(),
    }];
    let err = classify_cumulative(sql, &refs, &events_source_map(), false, &fds).unwrap_err();
    let diag = err
        .iter()
        .find(|d| matches!(d, KeyedDiagnostic::KeyedOnceWriteUnproven { .. }))
        .unwrap_or_else(|| panic!("expected KeyedOnceWriteUnproven: {err:?}"));
    let KeyedDiagnostic::KeyedOnceWriteUnproven { column, reason, .. } = diag else {
        unreachable!()
    };
    assert_eq!(column, "signup_referrer");
    assert!(
        reason.contains("fallback") && reason.contains("NULL"),
        "the reason must name the NULL-masking fallback: {reason}"
    );
    assert!(
        diag.to_string().contains("downstream"),
        "the refusal must name dropping the fallback / applying the default downstream: {diag}"
    );
}

/// The same NULL-masking hazard through a second candidate rather than a
/// literal: `COALESCE(MAX(a), MAX(b))` is NULL-preserving, but the merge
/// `COALESCE(target, delta)` does not preserve the argument PREFERENCE
/// order across windows — a window where `a IS NULL` but `b` is present
/// writes `b`'s value, which a later window carrying `a` can never
/// displace, while a full refresh prefers `a`. Refuse.
#[test]
fn once_write_with_a_second_candidate_argument_refuses() {
    let sql = r#"SELECT
    device_id,
    COALESCE(MAX(signup_referrer), MAX(fallback_referrer)) AS first_referrer
FROM smelt.silver.events_parsed
GROUP BY device_id"#;
    let refs = vec!["smelt.silver.events_parsed".to_string()];
    let fds = vec![
        FunctionalDependency {
            key: vec!["device_id".to_string()],
            determines: "signup_referrer".to_string(),
        },
        FunctionalDependency {
            key: vec!["device_id".to_string()],
            determines: "fallback_referrer".to_string(),
        },
    ];
    let err = classify_cumulative(sql, &refs, &events_source_map(), false, &fds).unwrap_err();
    assert!(
        err.iter().any(|d| matches!(
            d,
            KeyedDiagnostic::KeyedOnceWriteUnproven { column, .. } if column == "signup_referrer"
        )),
        "diagnostics: {err:?}"
    );
}

/// Route 1 keeps its fallback: a bare `unique_key` column reference is
/// non-null within its own group by construction, so `COALESCE(key, 'n/a')`
/// is a per-key constant no later window can contradict — the fallback can
/// never mask a later observation, and the shape stays admitted.
#[test]
fn key_derived_once_write_keeps_its_literal_fallback() {
    let sql = r#"SELECT
    device_id,
    COALESCE(device_id, 'n/a') AS first_seen_device
FROM smelt.silver.events_parsed
GROUP BY device_id"#;
    let refs = vec!["smelt.silver.events_parsed".to_string()];
    let classification = classify_cumulative(sql, &refs, &events_source_map(), false, &[])
        .expect("a key-derived COALESCE keeps its fallback");
    let col = classification
        .aggregator_columns
        .iter()
        .find(|c| c.output_name == "first_seen_device")
        .expect("first_seen_device column present");
    assert_eq!(
        col.cross_partition_combiner,
        CrossPartitionCombiner::OnceWrite
    );
}

/// Once-write refuses snapshot-reconcile (observer semantics): a
/// `COALESCE`-shaped once-write column under zero clocked sources refuses
/// `KeyedSnapshotSourceUnsupportedColumn` naming the "once-write" family,
/// even when it would otherwise be admitted (key-derived, no proof needed).
#[test]
fn once_write_refuses_snapshot_reconcile() {
    let sql = r#"SELECT
    device_id,
    COALESCE(device_id, 'n/a') AS first_seen_device
FROM smelt.silver.lookup_table
GROUP BY device_id"#;
    let refs = vec!["smelt.silver.lookup_table".to_string()];
    let err = classify_cumulative(sql, &refs, &HashMap::new(), false, &[]).unwrap_err();
    assert!(
        err.iter().any(|d| matches!(
            d,
            KeyedDiagnostic::KeyedSnapshotSourceUnsupportedColumn { family, .. }
                if family == "once-write"
        )),
        "diagnostics: {err:?}"
    );
}

/// `CrossPartitionCombiner::OnceWrite` renders `COALESCE(target, delta)`.
#[test]
fn once_write_combiner_renders_coalesce() {
    let rendered =
        CrossPartitionCombiner::OnceWrite.render("target.signup_referrer", "delta.signup_referrer");
    assert_eq!(
        rendered,
        "COALESCE(target.signup_referrer, delta.signup_referrer)"
    );
}
