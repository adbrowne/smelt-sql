use std::collections::HashMap;

use anyhow::Result;
use chrono::NaiveDate;

use smelt_core::config::Config;
use smelt_core::graph::DependencyGraph;

use crate::types::ExecuteRequest;

/// Parse the run's `[start, end)` event-time window from the request. End is
/// exclusive; both must be present together or neither. Extracted so the
/// dry-run statement-emission branch and the real run resolve the window
/// identically (`docs/specs/cli.md` §"`--dry-run` prints the maintenance
/// statements": region literals are real, from this same resolved window).
pub(crate) fn parse_run_window(
    request: &ExecuteRequest,
) -> Result<(Option<NaiveDate>, Option<NaiveDate>)> {
    match (request.start.as_deref(), request.end.as_deref()) {
        (Some(s), Some(e)) => {
            match (
                NaiveDate::parse_from_str(s, "%Y-%m-%d"),
                NaiveDate::parse_from_str(e, "%Y-%m-%d"),
            ) {
                (Ok(sd), Ok(ed)) => {
                    if sd >= ed {
                        anyhow::bail!("Start date must be before end date");
                    }
                    Ok((Some(sd), Some(ed)))
                }
                // Not calendar-shaped — the run window may still be valid on
                // an integer partition axis (`docs/specs/timeseries.md`
                // §"Validation rules" rule 9). This function only serves the
                // calendar-axis-only consumers of `start_date`/`end_date`
                // (key-grain dispatch, `PartitionRange` construction, etc.);
                // `build_model_plans` re-resolves the raw `request.start`/
                // `request.end` strings per model against that model's own
                // resolved axis (`resolve_partition_axes`), so returning
                // `(None, None)` here for a bare-integer pair does not lose
                // the window for those integer-axis models — it just means
                // this global NaiveDate view has nothing to say about it.
                // A pair that is neither calendar-shaped nor a valid
                // increasing integer pair is still a hard, fail-loud error.
                _ => match (s.parse::<i64>(), e.parse::<i64>()) {
                    (Ok(si), Ok(ei)) if si < ei => Ok((None, None)),
                    (Ok(_), Ok(_)) => {
                        anyhow::bail!("Start must be before end (got start={s}, end={e})")
                    }
                    _ => {
                        anyhow::bail!(
                            "Invalid time range bounds '{s}'/'{e}': expected both to be \
                             calendar dates (YYYY-MM-DD) or both to be bare integers (for an \
                             integer partition axis)"
                        )
                    }
                },
            }
        }
        (None, None) => Ok((None, None)),
        _ => anyhow::bail!("Both start and end must be provided together (or neither)"),
    }
}

/// Parse `request.start`/`request.end` (raw strings) against a specific
/// model's resolved partition axis — the generalization of
/// [`parse_run_window`] task 7 calls for. Unlike `parse_run_window` (which
/// serves every calendar-only consumer of the global `start_date`/`end_date`
/// pair), this is called once per selected partition-grain model inside
/// [`build_model_plans`], so a run mixing a calendar-axis model and an
/// integer-axis model resolves each bound correctly for its own model
/// rather than forcing one global domain.
pub(crate) fn parse_run_window_in_axis(
    request: &ExecuteRequest,
    axis: smelt_logical::PartitionAxis,
) -> Result<
    Option<(
        crate::windowing::PartitionPoint,
        crate::windowing::PartitionPoint,
    )>,
> {
    match (request.start.as_deref(), request.end.as_deref()) {
        (Some(s), Some(e)) => {
            let start = crate::windowing::PartitionPoint::parse_in_axis(s, axis)
                .map_err(|msg| anyhow::anyhow!("{msg}"))?;
            let end = crate::windowing::PartitionPoint::parse_in_axis(e, axis)
                .map_err(|msg| anyhow::anyhow!("{msg}"))?;
            Ok(Some((start, end)))
        }
        (None, None) => Ok(None),
        _ => anyhow::bail!("Both start and end must be provided together (or neither)"),
    }
}

/// Resolve each selected model's `partition_column` axis from its output
/// schema (`docs/specs/timeseries.md` §"Validation rules" rule 9). Reuses
/// the same `smelt_db::resolved_model_schema` Salsa read
/// `UpstreamSchemas::from_database` performs, rather than building a second
/// `UpstreamSchemas` — the schema this reads is the same one that read
/// would compute. A model absent from the returned map means the type
/// could not be resolved (no `timeseries:`, unresolved column, or
/// `Unknown` type); `build_model_plans` falls back for that model to the
/// axis implied by the run-window literal's own form, with a
/// `tracing::warn!` — an undecidable type is not a positive disproof of
/// either domain, matching the existing fail-open posture of
/// `derive_partition_grid_unit`.
pub(crate) fn resolve_partition_axes(
    db: &smelt_db::Database,
    selected: &[String],
    graph_lock: &DependencyGraph,
    config: &Config,
) -> HashMap<String, smelt_logical::PartitionAxis> {
    let mut out = HashMap::new();
    let Some(workspace) = smelt_db::Workspace::try_get(db) else {
        return out;
    };
    for model_name in selected {
        let Ok(model) = graph_lock.get_model(model_name) else {
            continue;
        };
        let metadata = model.metadata.as_deref();
        let ts_config = config
            .get_timeseries_with_metadata(model_name, metadata)
            .cloned()
            .or_else(|| metadata.and_then(|m| m.timeseries.clone()));
        let Some(ts) = ts_config else {
            continue;
        };
        let Some(file) = db.source_file(&model.path) else {
            continue;
        };
        let resolved = smelt_db::resolved_model_schema(db, workspace, file);
        let Some(col) = resolved
            .columns
            .iter()
            .find(|c| c.name == ts.partition_column)
        else {
            continue;
        };
        let Some(typed) = &col.data_type else {
            continue;
        };
        if let Some(axis) = smelt_logical::partition_axis_for_type(&typed.data_type) {
            out.insert(model_name.clone(), axis);
        }
    }
    out
}

/// Resolve the effective run-window bounds for one model's axis. Calendar
/// axis reads the already-globally-parsed `start_date`/`end_date`
/// (unchanged — every existing calendar-axis model resolves its window
/// exactly as before parse_run_window's leniency was added). Integer axis
/// re-parses `request.start`/`request.end` (the raw strings) directly
/// against that axis, since the global calendar-only parse in
/// [`parse_run_window`] returns `(None, None)` for a non-calendar-shaped
/// pair. `None` means no run window was supplied for this model at all;
/// `Err` means one was supplied but doesn't parse in this model's axis
/// (fail-closed, `docs/specs/incremental_shapes.md` §"The partition grain"
/// rule 8a).
pub(crate) fn window_for_axis(
    axis: smelt_logical::PartitionAxis,
    start_date: Option<NaiveDate>,
    end_date: Option<NaiveDate>,
    request: &ExecuteRequest,
) -> Result<
    Option<(
        crate::windowing::PartitionPoint,
        crate::windowing::PartitionPoint,
    )>,
> {
    match axis {
        smelt_logical::PartitionAxis::Calendar => Ok(match (start_date, end_date) {
            (Some(s), Some(e)) => Some((
                crate::windowing::PartitionPoint::Date(s),
                crate::windowing::PartitionPoint::Date(e),
            )),
            _ => None,
        }),
        smelt_logical::PartitionAxis::Integer => parse_run_window_in_axis(request, axis),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `docs/outcomes/20260815-partition-grain-residue/phases/05a-plan.md`
    /// §Tests — `parse_run_window_in_axis` (the generalization of
    /// `parse_run_window` task 7 calls for) parses `--event-time-start 1
    /// --event-time-end 4` into two `PartitionPoint::Integer`s when the
    /// resolved axis is `Integer`.
    fn test_request(start: &str, end: &str) -> ExecuteRequest {
        ExecuteRequest {
            target: "dev".to_string(),
            select: vec![],
            exclude: vec![],
            start: Some(start.to_string()),
            end: Some(end.to_string()),
            batch_size_days: None,
            per_partition: false,
            full_refresh: false,
            dry_run: true,
            enforce_safety: false,
            allow_column_removal: false,
            allow_full_refresh: false,
            ephemeral_seed_ctes: vec![],
            run_checks: false,
            checks: vec![],
            jobs: None,
            retry_max: None,
            retry_backoff_ms: None,
            resume: false,
            technique_overrides: vec![],
        }
    }

    #[test]
    fn parse_run_window_accepts_integer_bounds() {
        let request = test_request("1", "4");
        let (start, end) =
            parse_run_window_in_axis(&request, smelt_logical::PartitionAxis::Integer)
                .expect("integer bounds must parse")
                .expect("both bounds supplied");
        assert_eq!(start, crate::windowing::PartitionPoint::Integer(1));
        assert_eq!(end, crate::windowing::PartitionPoint::Integer(4));
    }

    /// `docs/outcomes/20260815-partition-grain-residue/phases/05b-plan.md`
    /// §Tests — the `Region` built for an integer-axis batch (via the same
    /// `Region::for_axis(batch.partition_start.axis(), ...)` call
    /// `execute.rs`'s batch loop makes) carries bare literals, not quoted
    /// ones.
    #[test]
    fn integer_axis_region_is_bare() {
        let batch = crate::windowing::PartitionPoint::Integer(1);
        let region = smelt_logical::maintenance::emit::Region::for_axis(
            batch.axis(),
            &batch.to_string(),
            &crate::windowing::PartitionPoint::Integer(2).to_string(),
        )
        .expect("integer literals must render");
        assert_eq!(region.start, "1");
        assert_eq!(region.end, "2");
    }

    #[test]
    fn parse_run_window_in_axis_refuses_calendar_bounds_on_integer_axis() {
        let request = test_request("2026-01-01", "2026-01-04");
        let err = parse_run_window_in_axis(&request, smelt_logical::PartitionAxis::Integer)
            .expect_err("a calendar-shaped bound must be refused on an integer axis");
        assert!(
            err.to_string().contains("bare integer"),
            "error must explain the axis mismatch, got: {err}"
        );
    }
}
