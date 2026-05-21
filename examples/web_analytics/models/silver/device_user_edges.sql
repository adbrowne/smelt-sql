---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
---
-- Per-day (device_id, user_id) co-occurrence evidence — every signed-in event
-- contributes one observation, aggregated to one row per (device, user, date).
-- The downstream view silver/device_user_edges_cumulative rolls these rows up
-- across all dates to reproduce the original cumulative edge shape consumed by
-- the gold/identity_backward_fill and gold/identity_connected_components
-- algorithms.  Splitting the aggregation into a per-day incremental table +
-- cumulative view keeps each daily run's compute proportional to that day's
-- signed-in events, rather than the full event history.
SELECT
    device_id,
    user_id,
    event_date,
    COUNT(*) AS daily_event_count,
    MIN(event_ts) AS daily_first_seen,
    MAX(event_ts) AS daily_last_seen
FROM smelt.silver.events_parsed
WHERE user_id IS NOT NULL
GROUP BY device_id, user_id, event_date
