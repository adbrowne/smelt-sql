//! `incremental_models.md` §"Key temporal locality (the time-partitioned
//! output)" — "The output as a clocked source": an admitted composed
//! (`grain: key` + `timeseries:`) output must be visible to the rest of the
//! DAG exactly like a declared source — downstream partition-grain models
//! get source-filter pushdown against it, and the clock does not stop at
//! the keyed stage (`docs/plans/20260715-composed-axes-conditional-
//! maintenance.md` Phase A5).
//!
//! `ModelGraph`/`ModelInfo` (`crate::graph`) carry no field distinguishing
//! "declared source" from "model output" — a model's own `timeseries_config`
//! is exactly what a source's would be. This test locks in that a
//! downstream model's pushdown derivation (`derive_model_source_bounds`)
//! treats an upstream composed model's clock identically to any other
//! clocked upstream.

use smelt_logical::graph::{ModelGraph, ModelInfo, TimeseriesConfig};
use smelt_logical::{derive_model_source_bounds, BoundResult, Granularity};

fn composed_upstream() -> ModelInfo {
    // Stands in for an admitted `grain: key` + `timeseries:` model
    // (`silver.events_deduped`, route 1 key-embedded): its own
    // `timeseries_config` is exactly what a declared source's would be —
    // `ModelGraph` records no provenance distinguishing the two.
    ModelInfo {
        name: "silver.events_deduped".to_string(),
        sql: String::new(),
        refs: vec![],
        timeseries_config: Some(TimeseriesConfig {
            event_time_column: "first_seen_at".to_string(),
            partition_column: "first_seen_date".to_string(),
            granularity: Granularity::Day,
            week_start: None,
            assert_monotonic: false,
        }),
        incremental_config: None,
        plausible_columns: Default::default(),
    }
}

/// A downstream partition-grain model with a genuine bounded lookback over
/// the composed model derives a nonzero pushdown margin from it, exactly as
/// it would from a declared source — the clock propagates through the
/// composed stage rather than degrading to a full scan.
#[test]
fn downstream_partition_grain_model_gets_pushdown_against_a_composed_upstream() {
    let mut graph = ModelGraph::new();
    graph.add_model(composed_upstream());

    let downstream = ModelInfo {
        name: "gold.daily_active_devices".to_string(),
        sql: "SELECT device_id, first_seen_date, COUNT(*) AS n \
              FROM silver.events_deduped \
              WHERE first_seen_date >= CAST(first_seen_date AS DATE) - INTERVAL '2 days' \
              GROUP BY device_id, first_seen_date"
            .to_string(),
        refs: vec!["silver.events_deduped".to_string()],
        timeseries_config: Some(TimeseriesConfig {
            event_time_column: "first_seen_at".to_string(),
            partition_column: "first_seen_date".to_string(),
            granularity: Granularity::Day,
            week_start: None,
            assert_monotonic: false,
        }),
        incremental_config: None,
        plausible_columns: Default::default(),
    };

    let bounds = derive_model_source_bounds(&downstream, &graph)
        .expect("bound must be derivable against a composed upstream");
    let (_, bound) = bounds
        .get("silver.events_deduped")
        .expect("the composed upstream must appear in the derived bounds — the clock-sink is gone");
    match bound {
        BoundResult::Bounded { before, after, .. } => {
            assert_eq!(*before, smelt_logical::Seconds::days(2));
            assert_eq!(*after, smelt_logical::Seconds::ZERO);
        }
        other => panic!("expected a Bounded pushdown window, got {other:?}"),
    }
}

/// Without any lookback construct the derived pushdown window is still
/// present (zero-margin), confirming the composed upstream is registered as
/// a source candidate at all — absence here would mean the downstream never
/// saw it as a clocked source (a clock-sink regression).
#[test]
fn downstream_partition_grain_model_sees_composed_upstream_even_with_no_lookback() {
    let mut graph = ModelGraph::new();
    graph.add_model(composed_upstream());

    let downstream = ModelInfo {
        name: "gold.daily_active_devices".to_string(),
        sql: "SELECT device_id, first_seen_date, COUNT(*) AS n \
              FROM silver.events_deduped GROUP BY device_id, first_seen_date"
            .to_string(),
        refs: vec!["silver.events_deduped".to_string()],
        timeseries_config: Some(TimeseriesConfig {
            event_time_column: "first_seen_at".to_string(),
            partition_column: "first_seen_date".to_string(),
            granularity: Granularity::Day,
            week_start: None,
            assert_monotonic: false,
        }),
        incremental_config: None,
        plausible_columns: Default::default(),
    };

    let bounds = derive_model_source_bounds(&downstream, &graph)
        .expect("bound must be derivable against a composed upstream");
    let (_, bound) = bounds
        .get("silver.events_deduped")
        .expect("the composed upstream must appear in the derived bounds");
    assert_eq!(
        *bound,
        BoundResult::Bounded {
            source_partition_col: "first_seen_date".to_string(),
            before: smelt_logical::Seconds::ZERO,
            after: smelt_logical::Seconds::ZERO,
        }
    );
}
