---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: session_start_date
  partition_column: session_start_date
  granularity: day
---
-- One row per session under the 30-minute inactivity + platform-boundary rule,
-- reconstructed across midnight from a bounded 1-day lookback.
--
-- The sessionization lives in the reusable `smelt.functions.sessionize`
-- transparent function: it assigns each event a stable `session_start_ts`
-- identity and declares its 1-day lookback via `RANGE BETWEEN INTERVAL` frames
-- in its body. The planner derives that bound from the expanded SQL and widens
-- the events_parsed read to the previous day, so a session whose events straddle
-- midnight is reconstructed as one row instead of being split at the partition
-- boundary.
--
-- session_id is (device_id, session_start_ts) — stable across run windows.
WITH sessionized AS (
    -- Columns projected explicitly: a TableExpr-returning function's output is
    -- opaque to the type checker, so the outer body names the columns it uses.
    SELECT
        device_id,
        event_ts,
        event_date,
        platform,
        session_start_ts,
        CAST(session_start_ts AS DATE) AS session_start_date
    FROM smelt.functions.sessionize(
        source => smelt.silver.events_parsed,
        partition_col => device_id,
        ts_col => event_ts,
        platform_col => platform
    )
)
-- Form B: the partition_column (session_start_date) is derived and skews earlier
-- than the events that update it. This filter declares event_date stays within
-- 1 day of session_start_date, so the planner rebases the WRITE window to
-- [D-1, D+1) and a cross-midnight session updates its prior-day partition.
SELECT
    CONCAT(CAST(device_id AS VARCHAR), '-', CAST(session_start_ts AS VARCHAR)) AS session_id,
    device_id,
    session_start_ts,
    session_start_date,
    MIN(event_ts) AS session_start,
    MAX(event_ts) AS session_end,
    COUNT(*) AS event_count,
    ANY_VALUE(platform) AS platform
FROM sessionized
WHERE event_date
    BETWEEN session_start_date - INTERVAL '1 day'
        AND session_start_date + INTERVAL '1 day'
GROUP BY device_id, session_start_ts, session_start_date
