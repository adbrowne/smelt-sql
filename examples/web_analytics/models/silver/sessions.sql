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
--
-- session_start_date — the partition_column — is computed via GROUP BY in the
-- session_starts CTE (one row per session with the earliest event's date) and
-- joined back into the outer SELECT.  This expresses the safety property
-- structurally: session_start_date is a function of (device_id, session_seq),
-- and the outer body groups by all three so the time-filter injection works.
-- No window function appears in the outer body; the LAG inside sessionize is
-- hidden by function expansion.
--
-- An earlier shape used FIRST_VALUE OVER inside a transparent function to
-- compute session_start_date; both forms produce identical output.  The
-- GROUP BY form is preferred because the safety property is visible in the
-- outer SQL.
WITH sessionized AS (
    -- Columns projected explicitly so the type checker resolves them on
    -- references from outer CTEs / SELECTs; SELECT * through a TableExpr-
    -- returning function is currently opaque to the type checker.
    SELECT
        device_id,
        event_ts,
        event_date,
        platform,
        session_seq
    FROM smelt.functions.sessionize(
        source => smelt.silver.events_parsed,
        partition_col => device_id,
        ts_col => event_ts,
        platform_col => platform
    )
),
session_starts AS (
    SELECT
        device_id,
        session_seq,
        CAST(MIN(event_ts) AS DATE) AS session_start_date
    FROM sessionized
    GROUP BY device_id, session_seq
)
SELECT
    CONCAT(CAST(s.device_id AS VARCHAR), '-', CAST(s.session_seq AS VARCHAR), '-', CAST(MIN(s.event_ts) AS VARCHAR)) AS session_id,
    s.device_id,
    s.session_seq,
    MIN(s.event_ts) AS session_start,
    MAX(s.event_ts) AS session_end,
    ss.session_start_date,
    COUNT(*) AS event_count,
    ANY_VALUE(s.platform) AS platform
FROM sessionized s
JOIN session_starts ss
    ON s.device_id = ss.device_id
   AND s.session_seq = ss.session_seq
GROUP BY s.device_id, s.session_seq, ss.session_start_date
