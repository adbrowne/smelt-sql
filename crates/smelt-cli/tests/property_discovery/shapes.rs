//! `EXPERIMENTAL(property-discovery): disposable`
//!
//! Local shape catalogue for the property-discovery cells whose construct
//! (self-referential models, `UNION ALL`, `LEFT JOIN`, correlated `EXISTS`,
//! stacked window frames, cross-source column-name collision, a mutable
//! source aggregated directly rather than joined, a cross-partition
//! `DISTINCT` CTE) has no equivalent in
//! `smelt_maintenance_testkit::recipe`'s typed `ModelRecipe` vocabulary
//! today (`docs/plans/20260712-generative-maintenance-conformance.md`
//! Phase 11's "anything the conformance gate does not subsume" carve-out).
//!
//! This module is a direct, unmodified copy of the shape-producing
//! functions the now-deleted `smelt_maintenance_testkit::model_shapes`
//! catalogue used to provide for these specific cells — retained here,
//! locally, so these probes stay independently compilable and executable
//! without the generative testkit's now-superseded disposable-generator
//! modules (`model_shapes.rs`, `run_schedule.rs`, both retired in favor of
//! `recipe.rs`/`schedule_gen.rs`).

/// Column declaration for a staged source (`name`, DuckDB type).
pub struct SourceColumn {
    pub name: &'static str,
    pub ty: &'static str,
}

/// A model shape: the model file SQL + the source it needs staged.
pub struct ModelShape {
    pub name: &'static str,
    pub sql: &'static str,
    pub source: &'static str,
    pub source_columns: &'static [SourceColumn],
}

/// A declared upstream source for a [`MultiSourceModelShape`].
pub struct MultiSourceSpec {
    pub name: &'static str,
    pub columns: &'static [SourceColumn],
    pub timeseries: Option<(&'static str, &'static str)>,
}

/// Like [`ModelShape`], but for cells whose model correlates more than one
/// staged source.
pub struct MultiSourceModelShape {
    pub name: &'static str,
    pub sql: &'static str,
    pub sources: &'static [MultiSourceSpec],
}

/// `SC-1`: correlated `EXISTS` 7-day attribution over two append-only
/// timeseries sources — `events` is the model's own driving source;
/// `conversions` is read only inside the correlated subquery's `WHERE`.
pub fn correlated_exists_attribution() -> MultiSourceModelShape {
    MultiSourceModelShape {
        name: "event_conversions",
        sql: r#"---
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
refresh: incremental
grain: partition
---
SELECT
  e.event_date,
  e.user_id,
  EXISTS(
    SELECT 1 FROM smelt.sources.conversions c
    WHERE c.user_id = e.user_id
      AND c.conversion_date BETWEEN e.event_date AND e.event_date + INTERVAL '7 days'
  ) AS converted
FROM smelt.sources.events e
"#,
        sources: &[
            MultiSourceSpec {
                name: "events",
                columns: &[
                    SourceColumn {
                        name: "event_date",
                        ty: "DATE",
                    },
                    SourceColumn {
                        name: "user_id",
                        ty: "BIGINT",
                    },
                ],
                timeseries: Some(("event_date", "event_date")),
            },
            MultiSourceSpec {
                name: "conversions",
                columns: &[
                    SourceColumn {
                        name: "user_id",
                        ty: "BIGINT",
                    },
                    SourceColumn {
                        name: "conversion_date",
                        ty: "DATE",
                    },
                ],
                timeseries: Some(("conversion_date", "conversion_date")),
            },
        ],
    }
}

/// `SC-2`: additive `SUM` group-by, batched per partition, over a clocked
/// source declared `mutation_profile: mutable`.
pub fn additive_agg_mutable_source() -> ModelShape {
    ModelShape {
        name: "events_daily_total",
        sql: r#"---
timeseries:
  event_time_column: d
  partition_column: d
  granularity: day
refresh: incremental
grain: partition
maintenance:
  scan_bounds:
    per_source:
      events:
        allow_full_scan: true
---
SELECT d, SUM(val) AS total FROM smelt.sources.events GROUP BY d
"#,
        source: "events",
        source_columns: &[
            SourceColumn {
                name: "d",
                ty: "DATE",
            },
            SourceColumn {
                name: "id",
                ty: "INTEGER",
            },
            SourceColumn {
                name: "val",
                ty: "DOUBLE",
            },
        ],
    }
}

/// `G-04`: idempotent `MIN` group-by, batched per partition, over a source
/// declared `mutation_profile: mutable` — idempotent-but-non-invertible,
/// distinct from `SC-2`'s additive/invertible combiner.
pub fn idempotent_agg_mutable_source() -> ModelShape {
    ModelShape {
        name: "events_daily_min_mutable",
        sql: r#"---
timeseries:
  event_time_column: d
  partition_column: d
  granularity: day
refresh: incremental
grain: partition
maintenance:
  scan_bounds:
    per_source:
      events:
        allow_full_scan: true
---
SELECT d, MIN(val) AS min_val FROM smelt.sources.events GROUP BY d
"#,
        source: "events",
        source_columns: &[
            SourceColumn {
                name: "d",
                ty: "DATE",
            },
            SourceColumn {
                name: "id",
                ty: "INTEGER",
            },
            SourceColumn {
                name: "val",
                ty: "DOUBLE",
            },
        ],
    }
}

/// `G-06`: left-join null-preservation (fact `events` × right-side
/// `refunds`), both declared append-only timeseries sources.
pub fn left_join_late_right_side() -> MultiSourceModelShape {
    MultiSourceModelShape {
        name: "events_with_refunds",
        sql: r#"---
timeseries:
  event_time_column: d
  partition_column: d
  granularity: day
refresh: incremental
grain: partition
---
SELECT e.d, e.user_id, e.val, r.refund_amt
FROM smelt.sources.events e
LEFT JOIN smelt.sources.refunds r ON e.user_id = r.user_id AND e.d = r.refund_date
"#,
        sources: &[
            MultiSourceSpec {
                name: "events",
                columns: &[
                    SourceColumn {
                        name: "d",
                        ty: "DATE",
                    },
                    SourceColumn {
                        name: "user_id",
                        ty: "BIGINT",
                    },
                    SourceColumn {
                        name: "val",
                        ty: "DOUBLE",
                    },
                ],
                timeseries: Some(("d", "d")),
            },
            MultiSourceSpec {
                name: "refunds",
                columns: &[
                    SourceColumn {
                        name: "refund_date",
                        ty: "DATE",
                    },
                    SourceColumn {
                        name: "user_id",
                        ty: "BIGINT",
                    },
                    SourceColumn {
                        name: "refund_amt",
                        ty: "DOUBLE",
                    },
                ],
                timeseries: Some(("refund_date", "refund_date")),
            },
        ],
    }
}

/// `G-08`: windowed running total via a self-referential batched model
/// (`docs/specs/incremental_shapes.md` §"Window independence and
/// self-referential models"), wrapped in a subquery.
pub fn running_balance_self_ref() -> ModelShape {
    ModelShape {
        name: "running_balance",
        sql: r#"---
timeseries:
  event_time_column: d
  partition_column: d
  granularity: day
refresh: incremental
grain: partition
maintenance:
  scan_bounds:
    per_source:
      transactions:
        allow_full_scan: true
---
SELECT d, balance FROM (
  SELECT
    t.d AS d,
    COALESCE(bal.balance, 0) + SUM(t.amt) AS balance
  FROM smelt.sources.transactions t
  LEFT JOIN smelt.running_balance bal
    ON bal.d >= t.d - INTERVAL '1 day' AND bal.d < t.d
  GROUP BY t.d, bal.balance
) inner_balance
"#,
        source: "transactions",
        source_columns: &[
            SourceColumn {
                name: "d",
                ty: "DATE",
            },
            SourceColumn {
                name: "amt",
                ty: "DOUBLE",
            },
        ],
    }
}

/// `G-11`: the same self-referential running-balance construct as
/// [`running_balance_self_ref`], but without the subquery wrap — a direct
/// self-join exposing the model's own output column (`d`) under its own
/// name from both the driving-source alias and the self-reference alias in
/// one `FROM` scope.
pub fn running_balance_self_ref_direct_join() -> ModelShape {
    ModelShape {
        name: "running_balance",
        sql: r#"---
timeseries:
  event_time_column: d
  partition_column: d
  granularity: day
refresh: incremental
grain: partition
maintenance:
  scan_bounds:
    per_source:
      transactions:
        allow_full_scan: true
---
SELECT
  t.d AS d,
  COALESCE(bal.balance, 0) + SUM(t.amt) AS balance
FROM smelt.sources.transactions t
LEFT JOIN smelt.running_balance bal
  ON bal.d >= t.d - INTERVAL '1 day' AND bal.d < t.d
GROUP BY t.d, bal.balance
"#,
        source: "transactions",
        source_columns: &[
            SourceColumn {
                name: "d",
                ty: "DATE",
            },
            SourceColumn {
                name: "amt",
                ty: "DOUBLE",
            },
        ],
    }
}

/// `G-09`: `UNION ALL` of two independent append-only timeseries sources.
pub fn union_all_two_append_only() -> MultiSourceModelShape {
    MultiSourceModelShape {
        name: "events_union_all",
        sql: r#"---
timeseries:
  event_time_column: d
  partition_column: d
  granularity: day
refresh: incremental
grain: partition
---
SELECT d, id, val, 'a' AS src FROM smelt.sources.events_a
UNION ALL
SELECT d, id, val, 'b' AS src FROM smelt.sources.events_b
"#,
        sources: &[
            MultiSourceSpec {
                name: "events_a",
                columns: &[
                    SourceColumn {
                        name: "d",
                        ty: "DATE",
                    },
                    SourceColumn {
                        name: "id",
                        ty: "INTEGER",
                    },
                    SourceColumn {
                        name: "val",
                        ty: "DOUBLE",
                    },
                ],
                timeseries: Some(("d", "d")),
            },
            MultiSourceSpec {
                name: "events_b",
                columns: &[
                    SourceColumn {
                        name: "d",
                        ty: "DATE",
                    },
                    SourceColumn {
                        name: "id",
                        ty: "INTEGER",
                    },
                    SourceColumn {
                        name: "val",
                        ty: "DOUBLE",
                    },
                ],
                timeseries: Some(("d", "d")),
            },
        ],
    }
}

/// `SC-1b`: two sources declaring partition columns with the same name
/// (`d`); a Form-B pattern textually scoped to one source's alias may still
/// satisfy the LHS name check for the other.
pub fn column_name_collision_across_sources() -> MultiSourceModelShape {
    MultiSourceModelShape {
        name: "logins_with_reset_flag",
        sql: r#"---
timeseries:
  event_time_column: d
  partition_column: d
  granularity: day
refresh: incremental
grain: partition
---
SELECT
  l.d,
  l.user_id,
  EXISTS(
    SELECT 1 FROM smelt.sources.resets r
    WHERE r.user_id = l.user_id
      AND r.d BETWEEN l.d AND l.d + INTERVAL '3 days'
  ) AS reset_flag
FROM smelt.sources.logins l
"#,
        sources: &[
            MultiSourceSpec {
                name: "logins",
                columns: &[
                    SourceColumn {
                        name: "d",
                        ty: "DATE",
                    },
                    SourceColumn {
                        name: "user_id",
                        ty: "BIGINT",
                    },
                ],
                timeseries: Some(("d", "d")),
            },
            MultiSourceSpec {
                name: "resets",
                columns: &[
                    SourceColumn {
                        name: "user_id",
                        ty: "BIGINT",
                    },
                    SourceColumn {
                        name: "d",
                        ty: "DATE",
                    },
                ],
                timeseries: Some(("d", "d")),
            },
        ],
    }
}

/// `SC-7`: a cross-partition `DISTINCT` inside a CTE body, consumed by an
/// outer aligned per-row query.
pub fn cte_cross_partition_distinct() -> ModelShape {
    ModelShape {
        name: "events_tiered",
        sql: r#"---
timeseries:
  event_time_column: d
  partition_column: d
  granularity: day
refresh: incremental
grain: partition
---
WITH user_tiers AS (
    SELECT DISTINCT user_id, tier FROM smelt.sources.events
)
SELECT e.d, e.user_id, t.tier
FROM smelt.sources.events e
JOIN user_tiers t ON e.user_id = t.user_id
"#,
        source: "events",
        source_columns: &[
            SourceColumn {
                name: "d",
                ty: "DATE",
            },
            SourceColumn {
                name: "user_id",
                ty: "BIGINT",
            },
            SourceColumn {
                name: "tier",
                ty: "VARCHAR",
            },
        ],
    }
}

/// `SC-4`: stacked bounded `RANGE` frames across CTE layers — the true
/// backward reach is the series sum (10 days), not the per-frame max (7).
pub fn stacked_range_frames() -> ModelShape {
    ModelShape {
        name: "metrics_stacked",
        sql: r#"---
timeseries:
  event_time_column: d
  partition_column: d
  granularity: day
refresh: incremental
grain: partition
---
WITH seven AS (
    SELECT
        d,
        SUM(v) OVER (ORDER BY d RANGE BETWEEN INTERVAL '7 days' PRECEDING AND CURRENT ROW) AS s7
    FROM smelt.sources.metrics
)
SELECT
    d,
    MAX(s7) OVER (ORDER BY d RANGE BETWEEN INTERVAL '3 days' PRECEDING AND CURRENT ROW) AS m3
FROM seven
"#,
        source: "metrics",
        source_columns: &[
            SourceColumn {
                name: "d",
                ty: "DATE",
            },
            SourceColumn {
                name: "v",
                ty: "DOUBLE",
            },
        ],
    }
}
