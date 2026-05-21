//! Backfill intelligence for incremental models.
//!
//! Provides batch generation, safety-to-batch-size mapping, and DAG-aware
//! range computation for backfill and backbuild operations.

use std::collections::HashMap;

use anyhow::{Context, Result};
use chrono::{Duration, NaiveDate};
use smelt_core::config::TimeseriesConfig;
use smelt_core::{Granularity, IncrementalConfig};
use smelt_planner::{analyze_batch_safety, BatchSafety, ModelInfo};

use crate::logical_graph::LogicalGraph;
use crate::temporal::compute_incremental_windows;
use crate::transformer::TimeRange;

/// A computed execution plan for one model during a backfill/backbuild.
#[derive(Debug, Clone)]
pub struct ModelBackfillPlan {
    /// Model name.
    pub model_name: String,
    /// The partition range — what gets written (DELETE/overwrite scope).
    pub partition_range: TimeRange,
    /// The filter range — what gets read (wider, includes context rows).
    pub filter_range: TimeRange,
    /// The batch safety classification for this model.
    pub batch_safety: BatchSafety,
    /// The batches to execute (each is a partition range).
    pub batches: Vec<BackfillBatch>,
    /// Whether this model is incremental (false = full refresh).
    pub is_incremental: bool,
}

/// A single batch within a backfill — covers a contiguous time range.
#[derive(Debug, Clone)]
pub struct BackfillBatch {
    /// The partition range for this batch (what gets written).
    pub partition_range: TimeRange,
    /// The filter range for this batch (what gets read — wider for context).
    pub filter_range: TimeRange,
}

/// Options controlling backfill execution.
#[derive(Debug, Clone, Default)]
pub struct BackfillOptions {
    /// Override batch size in days (takes precedence over safety analysis).
    pub batch_size_days: Option<u32>,
    /// Force per-partition execution (one query per granularity period).
    pub per_partition: bool,
}

/// Compute backfill plans for models in execution order.
///
/// For a regular range run, all models get the same requested range.
/// Each model's batch strategy is determined by its batch safety analysis.
pub fn compute_range_run_plans(
    execution_order: &[String],
    graph: &LogicalGraph,
    sources: Option<&smelt_core::SourcesConfig>,
    requested_range: &TimeRange,
    options: &BackfillOptions,
) -> Result<Vec<ModelBackfillPlan>> {
    let mut plans = Vec::new();

    for model_name in execution_order {
        let node = graph.get_node(model_name)?;
        let model = &node.model_file;

        let inc_config = node.incremental.clone();

        let refs = graph.get_upstream(model_name);
        let ts_config = node.timeseries.clone();
        let plan = match (inc_config, ts_config) {
            (Some(ref inc), Some(ref ts)) => compute_model_backfill_plan(
                model_name,
                &model.content,
                refs,
                inc,
                ts,
                sources,
                model.metadata.as_ref().map(|b| b.as_ref()),
                requested_range,
                options,
            )?,
            _ => ModelBackfillPlan {
                model_name: model_name.clone(),
                partition_range: requested_range.clone(),
                filter_range: requested_range.clone(),
                batch_safety: BatchSafety::FullyBatchSafe,
                batches: vec![],
                is_incremental: false,
            },
        };

        plans.push(plan);
    }

    Ok(plans)
}

/// Compute backfill plans for a backbuild operation.
///
/// Walks the DAG backwards from the target model, expanding ranges
/// based on each model's temporal dependencies and data latency.
pub fn compute_backbuild_plans(
    target_model: &str,
    execution_order: &[String],
    graph: &LogicalGraph,
    sources: Option<&smelt_core::SourcesConfig>,
    requested_range: &TimeRange,
    options: &BackfillOptions,
) -> Result<Vec<ModelBackfillPlan>> {
    // Build model ranges: start with the target's requested range,
    // then expand upstream ranges based on temporal dependencies.
    let mut model_ranges: HashMap<String, TimeRange> = HashMap::new();
    model_ranges.insert(target_model.to_string(), requested_range.clone());

    // Process in reverse execution order (downstream to upstream)
    let reversed: Vec<_> = execution_order.iter().rev().collect();

    for model_name in &reversed {
        let range = match model_ranges.get(model_name.as_str()) {
            Some(r) => r.clone(),
            None => continue,
        };

        let node = graph.get_node(model_name)?;
        let model = &node.model_file;

        let inc_config = node.incremental.clone();

        if let (Some(inc), Some(ts)) = (inc_config.as_ref(), node.timeseries.as_ref()) {
            // Compute the effective window for this model
            let model_latency = model
                .metadata
                .as_ref()
                .and_then(|m| m.columns.get(&ts.event_time_column))
                .and_then(|c| c.data_latency.as_ref());

            let windows = compute_incremental_windows(
                &model.content,
                inc,
                ts,
                sources,
                model_latency,
                &range,
            );

            // Each upstream must provide data for this model's filter range
            let upstream_range = &windows.filter_range;

            for upstream_name in graph.get_upstream(model_name) {
                let existing = model_ranges.get(&upstream_name);
                let expanded = match existing {
                    Some(existing_range) => union_ranges(existing_range, upstream_range)?,
                    None => upstream_range.clone(),
                };
                model_ranges.insert(upstream_name, expanded);
            }
        } else {
            // Non-incremental (or missing timeseries): upstreams still need to be included
            // (they'll get full refresh), use the same range for any upstream that is incremental
            for upstream_name in graph.get_upstream(model_name) {
                model_ranges.entry(upstream_name).or_insert(range.clone());
            }
        }
    }

    // Now compute backfill plans for each model in execution order
    let mut plans = Vec::new();

    for model_name in execution_order {
        let range = match model_ranges.get(model_name.as_str()) {
            Some(r) => r,
            None => continue,
        };

        let node = graph.get_node(model_name)?;
        let model = &node.model_file;

        let inc_config = node.incremental.clone();

        let refs = graph.get_upstream(model_name);
        let ts_config = node.timeseries.clone();
        let plan = match (inc_config, ts_config) {
            (Some(ref inc), Some(ref ts)) => compute_model_backfill_plan(
                model_name,
                &model.content,
                refs,
                inc,
                ts,
                sources,
                model.metadata.as_ref().map(|b| b.as_ref()),
                range,
                options,
            )?,
            _ => ModelBackfillPlan {
                model_name: model_name.clone(),
                partition_range: range.clone(),
                filter_range: range.clone(),
                batch_safety: BatchSafety::FullyBatchSafe,
                batches: vec![],
                is_incremental: false,
            },
        };

        plans.push(plan);
    }

    Ok(plans)
}

/// Compute the backfill plan for a single model.
#[allow(clippy::too_many_arguments)]
fn compute_model_backfill_plan(
    model_name: &str,
    sql: &str,
    refs: Vec<String>,
    inc_config: &IncrementalConfig,
    ts_config: &TimeseriesConfig,
    sources: Option<&smelt_core::SourcesConfig>,
    model_metadata: Option<&crate::metadata::ModelMetadata>,
    requested_range: &TimeRange,
    options: &BackfillOptions,
) -> Result<ModelBackfillPlan> {
    // Analyze batch safety
    let model_info = ModelInfo {
        name: model_name.to_string(),
        sql: sql.to_string(),
        refs,
        incremental_config: Some(inc_config.clone()),
        timeseries_config: Some(ts_config.clone()),
    };
    let batch_safety = analyze_batch_safety(&model_info);

    // Compute effective window
    let model_latency = model_metadata
        .and_then(|m| m.columns.get(&ts_config.event_time_column))
        .and_then(|c| c.data_latency.as_ref());
    let windows = compute_incremental_windows(
        sql,
        inc_config,
        ts_config,
        sources,
        model_latency,
        requested_range,
    );

    // Determine batch strategy
    let batches = generate_batches(
        requested_range,
        &windows.filter_range,
        &batch_safety,
        &ts_config.granularity,
        options,
    )?;

    Ok(ModelBackfillPlan {
        model_name: model_name.to_string(),
        partition_range: windows.partition_range,
        filter_range: windows.filter_range,
        batch_safety,
        batches,
        is_incremental: true,
    })
}

/// Compute batch safety and generate batches for a single model.
///
/// This is the per-model API used by the `run` command when a time range is provided.
/// It computes batch safety from the SQL, then generates batches accordingly.
pub fn compute_batches_for_model(
    sql: &str,
    inc_config: &IncrementalConfig,
    ts_config: &TimeseriesConfig,
    requested_range: &TimeRange,
    filter_range: &TimeRange,
    options: &BackfillOptions,
) -> Result<(BatchSafety, Vec<BackfillBatch>)> {
    let model_info = ModelInfo {
        name: String::new(),
        sql: sql.to_string(),
        refs: vec![],
        incremental_config: Some(inc_config.clone()),
        timeseries_config: Some(ts_config.clone()),
    };
    let batch_safety = analyze_batch_safety(&model_info);

    let batches = generate_batches(
        requested_range,
        filter_range,
        &batch_safety,
        &ts_config.granularity,
        options,
    )?;

    Ok((batch_safety, batches))
}

/// Generate batches for a backfill based on batch safety and options.
fn generate_batches(
    requested_range: &TimeRange,
    _filter_range: &TimeRange,
    batch_safety: &BatchSafety,
    granularity: &Granularity,
    options: &BackfillOptions,
) -> Result<Vec<BackfillBatch>> {
    let start = parse_date(&requested_range.start)?;
    let end = parse_date(&requested_range.end)?;

    if start >= end {
        return Ok(vec![]);
    }

    // Determine batch size in days
    let (batch_days, context_days) = if options.per_partition {
        // Force per-partition: one granularity period per batch
        let period = granularity_days(granularity);
        let context = match batch_safety {
            BatchSafety::BoundedSafe { context_days, .. } => *context_days,
            _ => 0,
        };
        (period, context)
    } else if let Some(override_days) = options.batch_size_days {
        let context = match batch_safety {
            BatchSafety::BoundedSafe { context_days, .. } => *context_days,
            _ => 0,
        };
        (override_days, context)
    } else {
        match batch_safety {
            BatchSafety::FullyBatchSafe => {
                // Single batch for entire range
                let total_days = (end - start).num_days() as u32;
                (total_days, 0)
            }
            BatchSafety::BoundedSafe {
                max_chunk_days,
                context_days,
                ..
            } => (*max_chunk_days, *context_days),
            BatchSafety::PerPartitionOnly { .. } => {
                let period = granularity_days(granularity);
                let context = 0; // per-partition doesn't need context across partitions
                (period, context)
            }
        }
    };

    // Generate batch ranges
    let mut batches = Vec::new();
    let mut batch_start = start;

    while batch_start < end {
        let batch_end = (batch_start + Duration::days(batch_days as i64)).min(end);

        let filter_start = batch_start - Duration::days(context_days as i64);
        let filter_end = batch_end;

        batches.push(BackfillBatch {
            partition_range: TimeRange {
                start: batch_start.format("%Y-%m-%d").to_string(),
                end: batch_end.format("%Y-%m-%d").to_string(),
            },
            filter_range: TimeRange {
                start: filter_start.format("%Y-%m-%d").to_string(),
                end: filter_end.format("%Y-%m-%d").to_string(),
            },
        });

        batch_start = batch_end;
    }

    Ok(batches)
}

/// Union two time ranges (take the min start and max end).
fn union_ranges(a: &TimeRange, b: &TimeRange) -> Result<TimeRange> {
    let a_start = parse_date(&a.start)?;
    let b_start = parse_date(&b.start)?;
    let a_end = parse_date(&a.end)?;
    let b_end = parse_date(&b.end)?;

    Ok(TimeRange {
        start: a_start.min(b_start).format("%Y-%m-%d").to_string(),
        end: a_end.max(b_end).format("%Y-%m-%d").to_string(),
    })
}

fn parse_date(s: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").with_context(|| format!("Invalid date format: {}", s))
}

/// Returns the batch period size in days for a given granularity.
///
/// For sub-day granularities (Hour), returns 1 day as the minimum batch unit
/// since batching operates at day boundaries.
fn granularity_days(g: &Granularity) -> u32 {
    match g {
        Granularity::Hour => 1, // Sub-day: batch at day boundaries
        Granularity::Day => 1,
        Granularity::Week => 7,
        Granularity::Month => 30,
        Granularity::Quarter => 91,
        Granularity::Year => 365,
    }
}

/// Format a backfill plan for dry-run display.
pub fn format_plan_summary(plans: &[ModelBackfillPlan]) -> String {
    let mut lines = Vec::new();

    for plan in plans {
        if !plan.is_incremental {
            lines.push(format!("  {} → full refresh", plan.model_name));
            continue;
        }

        let safety_label = match &plan.batch_safety {
            BatchSafety::FullyBatchSafe => "batch-safe".to_string(),
            BatchSafety::BoundedSafe {
                max_chunk_days,
                context_days,
                ..
            } => {
                format!(
                    "bounded ({}d chunks, {}d context)",
                    max_chunk_days, context_days
                )
            }
            BatchSafety::PerPartitionOnly { .. } => "per-partition".to_string(),
        };

        lines.push(format!(
            "  {} → {} batch(es), range [{}, {}), safety: {}",
            plan.model_name,
            plan.batches.len(),
            plan.partition_range.start,
            plan.partition_range.end,
            safety_label,
        ));

        if plan.filter_range.start != plan.partition_range.start
            || plan.filter_range.end != plan.partition_range.end
        {
            lines.push(format!(
                "    filter range: [{}, {}) (expanded for temporal context)",
                plan.filter_range.start, plan.filter_range.end,
            ));
        }
    }

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_batches_fully_safe() {
        let range = TimeRange {
            start: "2025-01-01".into(),
            end: "2026-01-01".into(),
        };
        let safety = BatchSafety::FullyBatchSafe;
        let options = BackfillOptions::default();

        let batches =
            generate_batches(&range, &range, &safety, &Granularity::Day, &options).unwrap();

        // FullyBatchSafe → single batch for entire range
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].partition_range.start, "2025-01-01");
        assert_eq!(batches[0].partition_range.end, "2026-01-01");
    }

    #[test]
    fn test_generate_batches_bounded_safe() {
        let range = TimeRange {
            start: "2025-01-01".into(),
            end: "2025-04-01".into(),
        };
        let safety = BatchSafety::BoundedSafe {
            max_chunk_days: 30,
            context_days: 7,
            reason: "7-day window lookback".into(),
        };
        let options = BackfillOptions::default();

        let batches =
            generate_batches(&range, &range, &safety, &Granularity::Day, &options).unwrap();

        // 90 days / 30-day chunks = 3 batches
        assert_eq!(batches.len(), 3);

        // First batch: partition [Jan 1, Jan 31), filter [Dec 25, Jan 31)
        assert_eq!(batches[0].partition_range.start, "2025-01-01");
        assert_eq!(batches[0].partition_range.end, "2025-01-31");
        assert_eq!(batches[0].filter_range.start, "2024-12-25");

        // Last batch: partition [Mar 2, Apr 1)
        assert_eq!(batches[2].partition_range.end, "2025-04-01");
    }

    #[test]
    fn test_generate_batches_per_partition() {
        let range = TimeRange {
            start: "2025-01-01".into(),
            end: "2025-01-04".into(),
        };
        let safety = BatchSafety::FullyBatchSafe;
        let options = BackfillOptions {
            per_partition: true,
            batch_size_days: None,
        };

        let batches =
            generate_batches(&range, &range, &safety, &Granularity::Day, &options).unwrap();

        // 3 days → 3 batches (per-partition forced)
        assert_eq!(batches.len(), 3);
        assert_eq!(batches[0].partition_range.start, "2025-01-01");
        assert_eq!(batches[0].partition_range.end, "2025-01-02");
        assert_eq!(batches[1].partition_range.start, "2025-01-02");
        assert_eq!(batches[1].partition_range.end, "2025-01-03");
        assert_eq!(batches[2].partition_range.start, "2025-01-03");
        assert_eq!(batches[2].partition_range.end, "2025-01-04");
    }

    #[test]
    fn test_generate_batches_override_size() {
        let range = TimeRange {
            start: "2025-01-01".into(),
            end: "2025-02-01".into(),
        };
        let safety = BatchSafety::FullyBatchSafe;
        let options = BackfillOptions {
            batch_size_days: Some(7),
            per_partition: false,
        };

        let batches =
            generate_batches(&range, &range, &safety, &Granularity::Day, &options).unwrap();

        // 31 days / 7 = 4 full + 1 partial = 5 batches
        assert_eq!(batches.len(), 5);
        assert_eq!(batches[0].partition_range.start, "2025-01-01");
        assert_eq!(batches[0].partition_range.end, "2025-01-08");
    }

    #[test]
    fn test_generate_batches_per_partition_only() {
        let range = TimeRange {
            start: "2025-01-01".into(),
            end: "2025-01-04".into(),
        };
        let safety = BatchSafety::PerPartitionOnly {
            reason: "unbounded lookback".into(),
        };
        let options = BackfillOptions::default();

        let batches =
            generate_batches(&range, &range, &safety, &Granularity::Day, &options).unwrap();

        // PerPartitionOnly → one batch per day
        assert_eq!(batches.len(), 3);
    }

    #[test]
    fn test_union_ranges() {
        let a = TimeRange {
            start: "2025-01-10".into(),
            end: "2025-03-01".into(),
        };
        let b = TimeRange {
            start: "2025-01-01".into(),
            end: "2025-02-15".into(),
        };
        let result = union_ranges(&a, &b).unwrap();
        assert_eq!(result.start, "2025-01-01");
        assert_eq!(result.end, "2025-03-01");
    }

    #[test]
    fn test_format_plan_summary_incremental() {
        let plans = vec![ModelBackfillPlan {
            model_name: "daily_revenue".into(),
            partition_range: TimeRange {
                start: "2025-01-01".into(),
                end: "2025-04-01".into(),
            },
            filter_range: TimeRange {
                start: "2024-12-25".into(),
                end: "2025-04-01".into(),
            },
            batch_safety: BatchSafety::BoundedSafe {
                max_chunk_days: 30,
                context_days: 7,
                reason: "7-day window".into(),
            },
            batches: vec![
                BackfillBatch {
                    partition_range: TimeRange {
                        start: "2025-01-01".into(),
                        end: "2025-01-31".into(),
                    },
                    filter_range: TimeRange {
                        start: "2024-12-25".into(),
                        end: "2025-01-31".into(),
                    },
                },
                BackfillBatch {
                    partition_range: TimeRange {
                        start: "2025-01-31".into(),
                        end: "2025-03-02".into(),
                    },
                    filter_range: TimeRange {
                        start: "2025-01-24".into(),
                        end: "2025-03-02".into(),
                    },
                },
            ],
            is_incremental: true,
        }];

        let summary = format_plan_summary(&plans);
        assert!(summary.contains("daily_revenue"));
        assert!(summary.contains("2 batch(es)"));
        assert!(summary.contains("bounded"));
        assert!(summary.contains("expanded for temporal context"));
    }

    #[test]
    fn test_format_plan_summary_full_refresh() {
        let plans = vec![ModelBackfillPlan {
            model_name: "staging_table".into(),
            partition_range: TimeRange {
                start: "2025-01-01".into(),
                end: "2025-04-01".into(),
            },
            filter_range: TimeRange {
                start: "2025-01-01".into(),
                end: "2025-04-01".into(),
            },
            batch_safety: BatchSafety::FullyBatchSafe,
            batches: vec![],
            is_incremental: false,
        }];

        let summary = format_plan_summary(&plans);
        assert!(summary.contains("full refresh"));
    }
}
