---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: session_start_date
  partition_column: session_start_date
  granularity: day
safety_overrides:
  allow_window_functions: true
---
-- One row per session under the 30-minute inactivity rule and the
-- **clock-anchored cut** (`functions/sessionize.sql`), ported from
-- `examples/web_analytics/silver/sessions.sql` — the web-analytics session
-- analogue in a different domain, borrowed so the two examples compare
-- directly. Nothing about GitHub activity argues for 30 minutes specifically
-- (`docs/outcomes/20260906-bigquery-dogfood-spine/phases/02-plan.md`).
--
-- Safe via the declared `RANGE BETWEEN INTERVAL '2 days' PRECEDING`
-- lookback frames inside `sessionize` and the explicit `HAVING` cap below,
-- not via partition-alignment — the override asserts that alternate safety
-- argument explicitly, matching `web_analytics/silver/sessions.sql`'s own
-- precedent.
WITH sessionized AS (
    SELECT
        actor_id,
        created_at AS event_ts,
        event_date,
        session_start_ts,
        CAST(session_start_ts AS DATE) AS session_start_date
    FROM smelt.functions.sessionize(
        source => smelt.silver.events_deduped,
        partition_col => actor_id,
        ts_col => created_at,
        platform_col => CAST(NULL AS VARCHAR)
    )
)
-- Form B: the partition_column (session_start_date) is the *earliest*
-- calendar day of the session, and the clock-anchored cut guarantees a
-- session's events land on that day or the next. This filter declares that
-- reach, so the planner rebases the WRITE window for a [D, D+1) run to
-- [D-1, D+2), and a cross-midnight session updates its prior-day partition.
SELECT
    CONCAT(CAST(actor_id AS VARCHAR), '-', CAST(session_start_ts AS VARCHAR)) AS session_id,
    actor_id,
    session_start_ts,
    session_start_date,
    MIN(event_ts) AS session_start,
    MAX(event_ts) AS session_end,
    COUNT(*) AS event_count
FROM sessionized
WHERE event_date
    BETWEEN session_start_date
        AND session_start_date + INTERVAL '1 day'
GROUP BY actor_id, session_start_ts, session_start_date
HAVING MAX(event_ts) - MIN(event_ts) < INTERVAL '2 days' -- max_lookback: explicit, checkable cap assertion
