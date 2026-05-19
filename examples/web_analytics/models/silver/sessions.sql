---
materialization: table
incremental:
  enabled: true
  event_time_column: session_start_date
  partition_column: session_start_date
  granularity: day
---
-- One row per session under the 30-minute inactivity + platform-boundary rule.
-- Delegates session-counter logic to smelt.functions.sessionize.
--
-- session_id is constructed via CONCAT rather than md5() because the smelt
-- type-inference layer recognizes CONCAT as a standard SQL function.
--
-- session_start_date is the incremental partition_column and must appear in
-- both the SELECT list and the GROUP BY as a real column (not an aggregate
-- alias) so that the optimizer can inject the incremental filter correctly.
-- FIRST_VALUE over the per-session event_ts ordering provides this value.
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
        CAST(FIRST_VALUE(event_ts) OVER (PARTITION BY device_id, session_seq ORDER BY event_ts) AS DATE) AS session_start_date
    FROM sessionized
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
