---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: session_start_date
  partition_column: session_start_date
  granularity: day
# `smelt.functions.sessionize`'s body computes session_start_ts (and hence
# session_start_date) via a window `OVER (PARTITION BY device_id,
# CAST(event_ts AS DATE))` — necessarily partitioned by the *input* event
# date, not by `session_start_date`, since the window's own job is to derive
# that column. Function-call expansion (phase 2, `docs/outcomes/
# 20260815-partition-grain-residue`) now classifies this window from the
# expanded body, where it was previously invisible to the alignment check.
# The model is safe via the declared `RANGE BETWEEN INTERVAL '2 days'
# PRECEDING` lookback frame and the explicit `HAVING` cap below (see the
# closed-form proof this file cites), not via partition-alignment — the
# override below asserts that alternate safety argument explicitly, matching
# `silver/events_parsed.sql`'s precedent for its own non-aligned window.
safety_overrides:
  allow_window_functions: true
---
-- One row per session under the 30-minute inactivity + platform-boundary rule
-- and the **clock-anchored cut**: a session rooted before 00:30 dies at its
-- own day's end; a session rooted at or after 00:30 may cross one midnight
-- but always dies at the *second* midnight. Every session spans at most two
-- calendar days (< 48h) — this is what keeps the table window-independent
-- (parallel, any-order partition builds) even under a never-idle input; see
-- `docs/research/20260711-clock-vs-root-anchored-sessions.md`
-- §"silver.sessions — clock-anchored cut" for the closed-form proof.
--
-- The sessionization lives in the reusable `smelt.functions.sessionize`
-- transparent function: it assigns each event a stable `session_start_ts`
-- identity and declares its lookback via `RANGE BETWEEN INTERVAL '2 days'
-- PRECEDING` frames in its body — `max_lookback`, named in the comments here
-- and in the function. The planner derives that bound from the expanded SQL
-- and widens the events_deduped read accordingly, so a session whose events
-- straddle midnight is reconstructed as one row instead of being split at
-- the partition boundary.
--
-- session_id is (device_id, session_start_ts) — stable across run windows.
--
-- The clock-anchored cut also seals old partitions for safe partition-level
-- maintenance: a session cannot span more than two calendar days, so a
-- partition older than that (plus the attribution window below) can never be
-- touched by a later event. A window-frame bound must be a literal
-- `INTERVAL '...'` (the grammar does not admit a parameter reference there),
-- so the cap cannot be threaded through as a `sessionize` argument; the
-- `HAVING` clause below restates it as an explicit, checkable assertion
-- instead — the actual per-session duration this model emits can never
-- exceed it, in the emitted SQL rather than only in the window-frame
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
        source => smelt.silver.events_deduped,
        partition_col => device_id,
        ts_col => event_ts,
        platform_col => platform
    )
)
-- Form B: the partition_column (session_start_date) is the *earliest*
-- calendar day of the session, and the clock-anchored cut guarantees a
-- session's events land on that day or the next — never earlier, never more
-- than one day later. This filter declares that reach, so the planner
-- rebases the WRITE window for a [D, D+1) run to [D-1, D+2) — half-open,
-- covering partitions D-1, D, and D+1 — and a cross-midnight session updates
-- its prior-day partition. The `1 day` here is the max-session-span cap
-- (< 48h total, since session_start_date to session_start_date + 1 day spans
-- two calendar days) restated as a date-column filter, because the
-- planner's Form B bound derivation works over the outer model's date-typed
-- partition column, not inside the function body; it must agree with the
-- `HAVING` assertion below and the function's `max_lookback` frames.
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
    BETWEEN session_start_date
        AND session_start_date + INTERVAL '1 day'
GROUP BY device_id, session_start_ts, session_start_date
HAVING MAX(event_ts) - MIN(event_ts) < INTERVAL '2 days' -- max_session_span: explicit, checkable cap assertion
