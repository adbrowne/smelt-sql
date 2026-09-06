use super::*;
use crate::analysis::walk::QueryTree;

fn fixture_ctx() -> SuccessionContext {
    SuccessionContext {
        source_name: "raw.customer_changes".to_string(),
        mutation_profile: Some(MutationProfile::AppendOnly),
        event_time_column: Some("changed_at".to_string()),
        not_null_columns: ["customer_id", "changed_at", "is_deleted"]
            .into_iter()
            .map(String::from)
            .collect(),
    }
}

fn classify(sql: &str, ctx: &SuccessionContext) -> SuccessionVerdict {
    let tree = QueryTree::from_sql(sql).expect("sql parses to a query tree");
    let crate::analysis::walk::QueryNode::Select(node) = &tree.root else {
        panic!("expected a top-level SELECT scope, got {:?}", tree.root);
    };
    classify_keyed_succession(node, ctx)
}

fn assert_recognized(verdict: &SuccessionVerdict) {
    assert!(
        matches!(verdict, SuccessionVerdict::Recognized { .. }),
        "expected Recognized, got {verdict:?}"
    );
}

fn assert_refused_as(verdict: &SuccessionVerdict, expected: fn(&NotSuccessionReason) -> bool) {
    match verdict {
        SuccessionVerdict::NotSuccession { reason } => {
            assert!(expected(reason), "unexpected refusal reason: {reason:?}");
        }
        other => panic!("expected NotSuccession, got {other:?}"),
    }
}

// ----- Recognition -----

#[test]
fn recognizes_minimal_lead_shape() {
    let sql = "SELECT customer_id, changed_at, \
                LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_ts \
                FROM smelt.raw.customer_changes";
    let verdict = classify(sql, &fixture_ctx());
    match &verdict {
        SuccessionVerdict::Recognized {
            lead_cols,
            lag_cols,
            delete_flag,
            pre_filter,
            key_cols,
            clock_col,
            ..
        } => {
            assert_eq!(lead_cols, &["next_ts".to_string()]);
            assert!(lag_cols.is_empty());
            assert_eq!(*delete_flag, None);
            assert_eq!(*pre_filter, None);
            assert_eq!(key_cols, &["customer_id".to_string()]);
            assert_eq!(clock_col, "changed_at");
        }
        other => panic!("expected Recognized, got {other:?}"),
    }
}

#[test]
fn recognizes_lag_projection() {
    let sql = "SELECT customer_id, changed_at, \
                LAG(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS prev_ts \
                FROM smelt.raw.customer_changes";
    let verdict = classify(sql, &fixture_ctx());
    match &verdict {
        SuccessionVerdict::Recognized {
            lag_cols,
            lead_cols,
            ..
        } => {
            assert_eq!(lag_cols, &["prev_ts".to_string()]);
            assert!(lead_cols.is_empty());
        }
        other => panic!("expected Recognized, got {other:?}"),
    }
}

#[test]
fn recognizes_scalar_expression_over_lead() {
    let sql = "SELECT customer_id, changed_at, \
                LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) IS NULL AS is_current \
                FROM smelt.raw.customer_changes";
    assert_recognized(&classify(sql, &fixture_ctx()));
}

#[test]
fn recognizes_qualify_not_flag_as_delete_flag() {
    let sql = "SELECT customer_id, changed_at, is_deleted, \
                LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_ts \
                FROM smelt.raw.customer_changes QUALIFY NOT is_deleted";
    match classify(sql, &fixture_ctx()) {
        SuccessionVerdict::Recognized { delete_flag, .. } => {
            assert_eq!(delete_flag, Some("is_deleted".to_string()));
        }
        other => panic!("expected Recognized, got {other:?}"),
    }
}

#[test]
fn recognizes_pre_window_clamp_as_pre_filter() {
    let sql = "SELECT customer_id, changed_at, \
                LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_ts \
                FROM smelt.raw.customer_changes WHERE changed_at >= DATE '2026-01-01'";
    match classify(sql, &fixture_ctx()) {
        SuccessionVerdict::Recognized { pre_filter, .. } => {
            assert!(pre_filter.is_some());
        }
        other => panic!("expected Recognized, got {other:?}"),
    }
}

#[test]
fn bare_negated_flag_pre_filter_carries_advisory() {
    let with_filter = "SELECT customer_id, changed_at, \
                LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_ts \
                FROM smelt.raw.customer_changes WHERE NOT is_deleted";
    let without_filter = "SELECT customer_id, changed_at, \
                LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_ts \
                FROM smelt.raw.customer_changes";
    let with_verdict = classify(with_filter, &fixture_ctx());
    let without_verdict = classify(without_filter, &fixture_ctx());
    match (&with_verdict, &without_verdict) {
        (
            SuccessionVerdict::Recognized {
                advisories,
                pre_filter,
                key_cols: k1,
                clock_col: c1,
                lead_cols: l1,
                lag_cols: g1,
                delete_flag: d1,
                ..
            },
            SuccessionVerdict::Recognized {
                advisories: advisories2,
                key_cols: k2,
                clock_col: c2,
                lead_cols: l2,
                lag_cols: g2,
                delete_flag: d2,
                ..
            },
        ) => {
            assert_eq!(
                advisories,
                &vec![SuccessionAdvisory::PreFilterNegatesFlag {
                    column: "is_deleted".to_string()
                }]
            );
            assert!(pre_filter.is_some());
            assert!(advisories2.is_empty());
            assert_eq!((k1, c1, l1, g1, d1), (k2, c2, l2, g2, d2));
        }
        other => panic!("expected both Recognized, got {other:?}"),
    }
}

// ----- Refusals -----

#[test]
fn refuses_non_succession_window_function() {
    let sql = "SELECT customer_id, changed_at, \
                SUM(customer_id) OVER (PARTITION BY customer_id ORDER BY changed_at) AS total \
                FROM smelt.raw.customer_changes";
    assert_refused_as(&classify(sql, &fixture_ctx()), |r| {
        matches!(r, NotSuccessionReason::WindowFunctionNotLead(_))
    });
}

#[test]
fn refuses_lead_over_other_column() {
    let sql = "SELECT customer_id, changed_at, \
                LEAD(customer_id) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_id \
                FROM smelt.raw.customer_changes";
    assert_refused_as(&classify(sql, &fixture_ctx()), |r| {
        matches!(r, NotSuccessionReason::WindowFunctionNotLead(_))
    });
}

#[test]
fn refuses_lead_with_explicit_offset() {
    let sql = "SELECT customer_id, changed_at, \
                LEAD(changed_at, 2) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_ts \
                FROM smelt.raw.customer_changes";
    assert_refused_as(&classify(sql, &fixture_ctx()), |r| {
        matches!(r, NotSuccessionReason::WindowFunctionNotLead(_))
    });
}

#[test]
fn refuses_lead_with_default_argument() {
    let sql = "SELECT customer_id, changed_at, \
                LEAD(changed_at, 1, changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_ts \
                FROM smelt.raw.customer_changes";
    assert_refused_as(&classify(sql, &fixture_ctx()), |r| {
        matches!(r, NotSuccessionReason::WindowFunctionNotLead(_))
    });
}

#[test]
fn refuses_mixed_partition_keys() {
    let sql = "SELECT customer_id, changed_at, \
                LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_ts, \
                LAG(changed_at) OVER (PARTITION BY changed_at ORDER BY changed_at) AS prev_ts \
                FROM smelt.raw.customer_changes";
    assert_refused_as(&classify(sql, &fixture_ctx()), |r| {
        matches!(r, NotSuccessionReason::PartitionKeyMismatch(_))
    });
}

#[test]
fn refuses_nullable_key() {
    let mut ctx = fixture_ctx();
    ctx.not_null_columns.remove("customer_id");
    let sql = "SELECT customer_id, changed_at, \
                LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_ts \
                FROM smelt.raw.customer_changes";
    assert_refused_as(&classify(sql, &ctx), |r| {
        matches!(r, NotSuccessionReason::OrderNotMonotoneClock(_))
    });
}

#[test]
fn refuses_nullable_clock() {
    let mut ctx = fixture_ctx();
    ctx.not_null_columns.remove("changed_at");
    let sql = "SELECT customer_id, changed_at, \
                LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_ts \
                FROM smelt.raw.customer_changes";
    assert_refused_as(&classify(sql, &ctx), |r| {
        matches!(r, NotSuccessionReason::OrderNotMonotoneClock(_))
    });
}

#[test]
fn refuses_non_strict_clock() {
    let sql = "SELECT customer_id, changed_at, \
                LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY CAST(changed_at AS DATE)) AS next_ts \
                FROM smelt.raw.customer_changes";
    assert_refused_as(&classify(sql, &fixture_ctx()), |r| {
        matches!(r, NotSuccessionReason::OrderNotMonotoneClock(_))
    });
}

#[test]
fn refuses_clock_not_event_time_column() {
    let mut ctx = fixture_ctx();
    ctx.not_null_columns.insert("created_at".to_string());
    let sql = "SELECT customer_id, created_at, \
                LEAD(created_at) OVER (PARTITION BY customer_id ORDER BY created_at) AS next_ts \
                FROM smelt.raw.customer_changes";
    assert_refused_as(&classify(sql, &ctx), |r| {
        matches!(r, NotSuccessionReason::OrderNotMonotoneClock(_))
    });
}

#[test]
fn refuses_descending_order() {
    let sql = "SELECT customer_id, changed_at, \
                LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at DESC) AS next_ts \
                FROM smelt.raw.customer_changes";
    assert_refused_as(&classify(sql, &fixture_ctx()), |r| {
        matches!(r, NotSuccessionReason::OrderNotMonotoneClock(_))
    });
}

#[test]
fn refuses_second_sort_key() {
    let sql = "SELECT customer_id, changed_at, \
                LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at, customer_id) AS next_ts \
                FROM smelt.raw.customer_changes";
    assert_refused_as(&classify(sql, &fixture_ctx()), |r| {
        matches!(r, NotSuccessionReason::OrderNotMonotoneClock(_))
    });
}

#[test]
fn refuses_order_by_expression_not_bare_column() {
    let sql = "SELECT customer_id, changed_at, \
                LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at + 1) AS next_ts \
                FROM smelt.raw.customer_changes";
    assert_refused_as(&classify(sql, &fixture_ctx()), |r| {
        matches!(r, NotSuccessionReason::OrderNotMonotoneClock(_))
    });
}

#[test]
fn refuses_two_window_calls_in_one_projection() {
    let sql = "SELECT customer_id, changed_at, \
                COALESCE(LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at), \
                LAG(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at)) AS both \
                FROM smelt.raw.customer_changes";
    assert_refused_as(&classify(sql, &fixture_ctx()), |r| {
        matches!(r, NotSuccessionReason::WindowFunctionNotLead(_))
    });
}

#[test]
fn refuses_unprojected_key() {
    let sql = "SELECT changed_at, \
                LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_ts \
                FROM smelt.raw.customer_changes";
    assert_refused_as(&classify(sql, &fixture_ctx()), |r| {
        matches!(r, NotSuccessionReason::IdentityNotProjected(_))
    });
}

#[test]
fn refuses_unprojected_clock() {
    let sql = "SELECT customer_id, \
                LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_ts \
                FROM smelt.raw.customer_changes";
    assert_refused_as(&classify(sql, &fixture_ctx()), |r| {
        matches!(r, NotSuccessionReason::IdentityNotProjected(_))
    });
}

#[test]
fn refuses_aggregate_sibling_projection() {
    let sql = "SELECT customer_id, changed_at, COUNT(*) OVER () AS total, \
                LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_ts \
                FROM smelt.raw.customer_changes";
    assert_refused_as(&classify(sql, &fixture_ctx()), |r| {
        matches!(r, NotSuccessionReason::WindowFunctionNotLead(_))
    });
}

#[test]
fn refuses_non_row_local_projected_column() {
    let sql = "SELECT customer_id, changed_at, (SELECT MAX(x) FROM other) AS bad, \
                LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_ts \
                FROM smelt.raw.customer_changes";
    assert_refused_as(&classify(sql, &fixture_ctx()), |r| {
        matches!(r, NotSuccessionReason::RowLocalColumnViolation(_))
    });
}

#[test]
fn refuses_join_from() {
    let sql = "SELECT c.customer_id, c.changed_at, \
                LEAD(c.changed_at) OVER (PARTITION BY c.customer_id ORDER BY c.changed_at) AS next_ts \
                FROM smelt.raw.customer_changes c JOIN smelt.raw.other o ON c.customer_id = o.customer_id";
    assert_refused_as(&classify(sql, &fixture_ctx()), |r| {
        matches!(r, NotSuccessionReason::SingleSourceOnly(_))
    });
}

#[test]
fn refuses_cte_from() {
    let sql = "WITH c AS (SELECT * FROM smelt.raw.customer_changes) \
                SELECT customer_id, changed_at, \
                LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_ts \
                FROM c";
    assert_refused_as(&classify(sql, &fixture_ctx()), |r| {
        matches!(r, NotSuccessionReason::SingleSourceOnly(_))
    });
}

#[test]
fn refuses_subquery_from() {
    let sql = "SELECT customer_id, changed_at, \
                LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_ts \
                FROM (SELECT * FROM smelt.raw.customer_changes) t";
    assert_refused_as(&classify(sql, &fixture_ctx()), |r| {
        matches!(r, NotSuccessionReason::SingleSourceOnly(_))
    });
}

#[test]
fn refuses_mutable_source() {
    let mut ctx = fixture_ctx();
    ctx.mutation_profile = Some(MutationProfile::Mutable);
    let sql = "SELECT customer_id, changed_at, \
                LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_ts \
                FROM smelt.raw.customer_changes";
    assert_refused_as(&classify(sql, &ctx), |r| {
        matches!(r, NotSuccessionReason::DrivingSourceNotAppendOnly(_))
    });
}

#[test]
fn refuses_change_feed_source() {
    let mut ctx = fixture_ctx();
    ctx.mutation_profile = Some(MutationProfile::ChangeFeed);
    let sql = "SELECT customer_id, changed_at, \
                LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_ts \
                FROM smelt.raw.customer_changes";
    assert_refused_as(&classify(sql, &ctx), |r| {
        matches!(r, NotSuccessionReason::DrivingSourceNotAppendOnly(_))
    });
}

#[test]
fn refuses_undeclared_mutation_profile() {
    let mut ctx = fixture_ctx();
    ctx.mutation_profile = None;
    let sql = "SELECT customer_id, changed_at, \
                LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_ts \
                FROM smelt.raw.customer_changes";
    assert_refused_as(&classify(sql, &ctx), |r| {
        matches!(r, NotSuccessionReason::DrivingSourceNotAppendOnly(_))
    });
}

#[test]
fn refuses_unclocked_source() {
    let mut ctx = fixture_ctx();
    ctx.event_time_column = None;
    let sql = "SELECT customer_id, changed_at, \
                LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_ts \
                FROM smelt.raw.customer_changes";
    assert_refused_as(&classify(sql, &ctx), |r| {
        matches!(r, NotSuccessionReason::DrivingSourceNotAppendOnly(_))
    });
}

#[test]
fn refuses_non_row_local_pre_filter() {
    let sql = "SELECT customer_id, changed_at, \
                LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_ts \
                FROM smelt.raw.customer_changes WHERE changed_at >= (SELECT MIN(x) FROM other)";
    assert_refused_as(&classify(sql, &fixture_ctx()), |r| {
        matches!(r, NotSuccessionReason::PreFilterNotRowLocal(_))
    });
}

#[test]
fn refuses_nondeterministic_pre_filter() {
    let sql = "SELECT customer_id, changed_at, \
                LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_ts \
                FROM smelt.raw.customer_changes WHERE changed_at <= NOW()";
    assert_refused_as(&classify(sql, &fixture_ctx()), |r| {
        matches!(r, NotSuccessionReason::PreFilterNotRowLocal(_))
    });
}

#[test]
fn refuses_qualify_other_shape() {
    let sql = "SELECT customer_id, changed_at, is_deleted, \
                LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_ts \
                FROM smelt.raw.customer_changes QUALIFY is_deleted";
    assert_refused_as(&classify(sql, &fixture_ctx()), |r| {
        matches!(r, NotSuccessionReason::DeleteFilterMisplaced(_))
    });
}

#[test]
fn refuses_qualify_nullable_flag() {
    let sql = "SELECT customer_id, changed_at, is_active, \
                LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_ts \
                FROM smelt.raw.customer_changes QUALIFY NOT is_active";
    assert_refused_as(&classify(sql, &fixture_ctx()), |r| {
        matches!(r, NotSuccessionReason::DeleteFilterMisplaced(_))
    });
}

#[test]
fn refuses_distinct() {
    let sql = "SELECT DISTINCT customer_id, changed_at, \
                LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_ts \
                FROM smelt.raw.customer_changes";
    assert_refused_as(&classify(sql, &fixture_ctx()), |r| {
        matches!(r, NotSuccessionReason::PatternUnrecognized(_))
    });
}

#[test]
fn refuses_group_by() {
    let sql = "SELECT customer_id, changed_at, \
                LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_ts \
                FROM smelt.raw.customer_changes GROUP BY customer_id, changed_at";
    assert_refused_as(&classify(sql, &fixture_ctx()), |r| {
        matches!(r, NotSuccessionReason::PatternUnrecognized(_))
    });
}

#[test]
fn refuses_order_by() {
    let sql = "SELECT customer_id, changed_at, \
                LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_ts \
                FROM smelt.raw.customer_changes ORDER BY changed_at";
    assert_refused_as(&classify(sql, &fixture_ctx()), |r| {
        matches!(r, NotSuccessionReason::PatternUnrecognized(_))
    });
}

#[test]
fn refuses_limit() {
    let sql = "SELECT customer_id, changed_at, \
                LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_ts \
                FROM smelt.raw.customer_changes LIMIT 10";
    assert_refused_as(&classify(sql, &fixture_ctx()), |r| {
        matches!(r, NotSuccessionReason::PatternUnrecognized(_))
    });
}

#[test]
fn refuses_having() {
    // HAVING requires GROUP BY in real SQL, but the classifier's rule 1b
    // checks the clause's mere presence — refuse before any GROUP BY
    // check would even matter.
    let sql = "SELECT customer_id, changed_at, \
                LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_ts \
                FROM smelt.raw.customer_changes GROUP BY customer_id, changed_at HAVING COUNT(*) > 1";
    assert_refused_as(&classify(sql, &fixture_ctx()), |r| {
        matches!(r, NotSuccessionReason::PatternUnrecognized(_))
    });
}
