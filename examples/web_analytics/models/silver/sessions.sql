---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: session_start_date
  partition_column: session_start_date
  granularity: day
---
-- One row per session under the 30-minute inactivity + platform-boundary rule.
-- Both window expressions (LAG for boundary detection in sessionize, FIRST_VALUE
-- for the per-session start_date in compute_session_start_date) live inside
-- transparent functions, so this model's outer body has no OVER clause.  That
-- is required for the planner to classify it as FullyBatchSafe and execute it
-- as a real incremental DELETE+INSERT per partition; an outer OVER would trip
-- the safety check and silently downgrade the model to full-rebuild.
--
-- session_start_date is the partition_column; it must appear in both the
-- SELECT list and the GROUP BY for the optimizer to inject the time filter.
WITH sessionized AS (
    SELECT *
    FROM smelt.functions.sessionize(
        source => smelt.silver.events_parsed,
        partition_col => device_id,
        ts_col => event_ts,
        platform_col => platform
    )
),
with_start_date AS (
    SELECT
        device_id,
        event_ts,
        event_date,
        platform,
        session_seq,
        session_start_date
    FROM smelt.functions.compute_session_start_date(
        source => sessionized,
        partition_col => device_id,
        session_seq_col => session_seq,
        ts_col => event_ts
    )
)
SELECT
    CONCAT(CAST(device_id AS VARCHAR), '-', CAST(session_seq AS VARCHAR), '-', CAST(MIN(event_ts) AS VARCHAR)) AS session_id,
    device_id,
    session_seq,
    MIN(event_ts) AS session_start,
    MAX(event_ts) AS session_end,
    session_start_date,
    COUNT(*) AS event_count,
    ANY_VALUE(platform) AS platform
FROM with_start_date
GROUP BY device_id, session_seq, session_start_date
