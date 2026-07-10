---
materialization: table
refresh: incremental
grain: partition
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
-- identity and declares its lookback via `RANGE BETWEEN INTERVAL '1 day'
-- PRECEDING` frames in its body — the **max-session-length cap**, named
-- `max_session_length` in the comments here and in the function. The planner
-- derives that bound from the expanded SQL and widens the events_parsed read
-- accordingly, so a session whose events straddle midnight is reconstructed
-- as one row instead of being split at the partition boundary.
--
-- session_id is (device_id, session_start_ts) — stable across run windows.
--
-- `max_session_length` also seals old partitions for safe partition-level
-- maintenance: a session cannot span more than this interval, so a partition
-- older than `max_session_length` (plus the attribution window below) can
-- never be touched by a later event. A window-frame bound must be a literal
-- `INTERVAL '...'` (the grammar does not admit a parameter reference there),
-- so `max_session_length` cannot be threaded through as a `sessionize`
-- argument; the `HAVING` clause below restates it as an explicit, checkable
-- assertion instead — the actual per-session duration this model emits can
-- never exceed it, in the emitted SQL rather than only in the window-frame
-- mechanics that happen to enforce it.
WITH sessionized AS (
    -- Columns projected explicitly: a TableExpr-returning function's output is
    -- opaque to the type checker, so the outer body names the columns it uses.
    SELECT
        device_id,
        event_ts,
        event_date,
        platform,
        utm_campaign,
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
-- 1 day of session_start_date, so the planner rebases the WRITE window for a
-- [D, D+1) run to [D-1, D+2) — half-open, covering partitions D-1, D, and D+1 —
-- and a cross-midnight session updates its prior-day partition. The
-- `1 day` here must match `max_session_length` above — it is the same cap,
-- restated as a date-column filter because the planner's Form B bound
-- derivation works over the outer model's date-typed partition column, not
-- inside the function body.
SELECT
    CONCAT(CAST(device_id AS VARCHAR), '-', CAST(session_start_ts AS VARCHAR)) AS session_id,
    device_id,
    session_start_ts,
    session_start_date,
    MIN(event_ts) AS session_start,
    MAX(event_ts) AS session_end,
    COUNT(*) AS event_count,
    ANY_VALUE(platform) AS platform,
    -- Campaign attribution: the earliest non-NULL `utm_campaign` among the
    -- session's own events within the first 5 minutes of the session start
    -- (MIN_BY-style: `ARG_MAX` keyed by the *negated* timestamp picks the
    -- value at the smallest, i.e. earliest, `event_ts`). Events beyond the
    -- first 5 minutes never attribute a campaign, even if the session itself
    -- runs longer.
    ARG_MAX(utm_campaign, -epoch_us(event_ts)) FILTER (
        WHERE utm_campaign IS NOT NULL
            AND event_ts <= session_start_ts + INTERVAL '5 minutes'
    ) AS utm_campaign
FROM sessionized
WHERE event_date
    BETWEEN session_start_date - INTERVAL '1 day'
        AND session_start_date + INTERVAL '1 day'
GROUP BY device_id, session_start_ts, session_start_date
HAVING MAX(event_ts) - MIN(event_ts) <= INTERVAL '1 day' -- max_session_length: explicit, checkable cap assertion
