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
-- Bounded sessionization (stable identity, no cumulative counter):
--   1. marked   — LAG the previous event's ts/platform.
--   2. bounded  — flag a session boundary when the gap exceeds 30 minutes,
--                 the platform changes, or there is no in-frame predecessor.
--   3. assigned — each event's session identity is the most recent boundary
--                 timestamp at or before it (running MAX over a 1-day frame);
--                 session_start_date — the partition_column — is that ts's date.
--
-- The `RANGE BETWEEN INTERVAL '1 day' PRECEDING AND CURRENT ROW` frames are the
-- load-bearing declaration: the planner derives a 1-day lookback (Form A) and
-- widens the events_parsed read to span the previous day, so a session that
-- straddles midnight is reconstructed as one row instead of being split at the
-- partition boundary.  The 1-day frame is also the cap: a session whose
-- sub-30-minute activity chain runs longer than the lookback is split at the
-- frame edge rather than reading unbounded history.
--
-- session_id is (device_id, session_start_ts) — stable across run windows,
-- unlike a window-relative sequence number, so a session reprocessed in a
-- different window keeps the same id.
WITH marked AS (
    SELECT
        device_id,
        event_ts,
        event_date,
        platform,
        LAG(event_ts) OVER (
            PARTITION BY device_id ORDER BY event_ts
            RANGE BETWEEN INTERVAL '1 day' PRECEDING AND CURRENT ROW
        ) AS prev_ts,
        LAG(platform) OVER (
            PARTITION BY device_id ORDER BY event_ts
            RANGE BETWEEN INTERVAL '1 day' PRECEDING AND CURRENT ROW
        ) AS prev_platform
    FROM smelt.silver.events_parsed
),
bounded AS (
    SELECT
        device_id,
        event_ts,
        event_date,
        platform,
        CASE
            WHEN prev_ts IS NULL THEN event_ts
            WHEN epoch_us(event_ts) - epoch_us(prev_ts) > 30 * 60 * 1000000 THEN event_ts
            WHEN prev_platform != platform THEN event_ts
            ELSE NULL
        END AS boundary_ts
    FROM marked
),
assigned AS (
    SELECT
        device_id,
        event_ts,
        event_date,
        platform,
        COALESCE(
            MAX(boundary_ts) OVER (
                PARTITION BY device_id ORDER BY event_ts
                RANGE BETWEEN INTERVAL '1 day' PRECEDING AND CURRENT ROW
            ),
            event_ts
        ) AS session_start_ts,
        CAST(
            COALESCE(
                MAX(boundary_ts) OVER (
                    PARTITION BY device_id ORDER BY event_ts
                    RANGE BETWEEN INTERVAL '1 day' PRECEDING AND CURRENT ROW
                ),
                event_ts
            ) AS DATE
        ) AS session_start_date
    FROM bounded
)
-- Form B: the partition_column (session_start_date) is derived and skews earlier
-- than the events that update it — a session that starts on D-1 keeps receiving
-- events on D. This filter declares that event_date stays within 1 day of
-- session_start_date, so the planner rebases the WRITE window to [D-1, D+1) and
-- a cross-midnight session updates its prior-day partition instead of leaving a
-- stale, truncated row behind. (The frames above already widen the READ.)
SELECT
    CONCAT(CAST(device_id AS VARCHAR), '-', CAST(session_start_ts AS VARCHAR)) AS session_id,
    device_id,
    session_start_ts,
    session_start_date,
    MIN(event_ts) AS session_start,
    MAX(event_ts) AS session_end,
    COUNT(*) AS event_count,
    ANY_VALUE(platform) AS platform
FROM assigned
WHERE event_date
    BETWEEN session_start_date - INTERVAL '1 day'
        AND session_start_date + INTERVAL '1 day'
GROUP BY device_id, session_start_ts, session_start_date
