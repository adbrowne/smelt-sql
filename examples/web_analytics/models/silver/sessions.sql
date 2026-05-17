---
materialization: table
incremental:
  enabled: true
  event_time_column: session_start_date
  partition_column: session_start_date
  granularity: day
---
-- One row per session under the 30-minute inactivity + platform-boundary rule.
-- The sessionization logic is inlined (rather than calling
-- smelt.functions.sessionize) because column-reference arguments to smelt
-- functions are not yet supported in model contexts; the function declaration
-- in functions/sessionize.sql is the canonical signature for that future
-- refactor.
--
-- Two-CTE approach is required because DuckDB does not allow nested window
-- functions: the LAG calls must be resolved in a separate CTE before the
-- outer SUM window function can reference them.
--
-- session_id is constructed via CONCAT rather than md5() because the smelt
-- type-inference layer recognizes CONCAT as a standard SQL function.
--
-- The inactivity gap is expressed using epoch_us() arithmetic (microseconds)
-- rather than INTERVAL subtraction because DuckDB's TIMESTAMP arithmetic
-- produces a BIGINT (microseconds since epoch) when the upstream model derives
-- event_ts via DATE + to_seconds(), which avoids a type mismatch between the
-- BIGINT difference and an INTERVAL literal. 30 minutes = 30 * 60 * 1_000_000
-- microseconds.
WITH lagged AS (
    SELECT
        device_id,
        event_ts,
        event_date,
        platform,
        LAG(epoch_us(event_ts)) OVER (PARTITION BY device_id ORDER BY event_ts) AS prev_ts_us,
        LAG(platform) OVER (PARTITION BY device_id ORDER BY event_ts) AS prev_platform
    FROM smelt.silver.events_parsed
),
sessionized AS (
    SELECT
        device_id,
        event_ts,
        event_date,
        platform,
        SUM(
            CASE
                WHEN epoch_us(event_ts) - prev_ts_us > 30 * 60 * 1000000
                  OR prev_platform != platform
                THEN 1
                ELSE 0
            END
        ) OVER (PARTITION BY device_id ORDER BY event_ts) AS session_seq
    FROM lagged
)
SELECT
    CONCAT(CAST(device_id AS VARCHAR), '-', CAST(session_seq AS VARCHAR), '-', CAST(MIN(event_ts) AS VARCHAR)) AS session_id,
    device_id,
    session_seq,
    MIN(event_ts) AS session_start,
    MAX(event_ts) AS session_end,
    CAST(MIN(event_ts) AS DATE) AS session_start_date,
    COUNT(*) AS event_count,
    ANY_VALUE(platform) AS platform
FROM sessionized
GROUP BY device_id, session_seq
