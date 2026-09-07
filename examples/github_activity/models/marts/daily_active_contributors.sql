---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: session_start_date
  partition_column: session_start_date
  granularity: day
---
-- Per-day distinct-actor, session and event counts — the silver output made
-- visible as a product. `daily_active_contributors` over a leaderboard: the
-- sample skews hard to newly-created bot repos (92% `PushEvent`, median
-- repo has one event — see `README.md`), so a leaderboard mart would be
-- mostly noise; a daily-actives count degrades gracefully under that skew
-- (`docs/outcomes/20260906-bigquery-dogfood-spine/phases/02-plan.md`).
SELECT
    session_start_date,
    COUNT(*) AS total_sessions,
    COUNT(DISTINCT actor_id) AS distinct_actors,
    SUM(event_count) AS total_events
FROM smelt.silver.actor_sessions
GROUP BY session_start_date
ORDER BY session_start_date
