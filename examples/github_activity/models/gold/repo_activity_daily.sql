---
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
refresh: incremental
grain: partition
---
-- Per-(repo, day) event and distinct-actor counts, the fact table
-- `marts.repo_leaderboard` and `marts.star_growth` roll up
-- (`docs/outcomes/20260906-bigquery-dogfood-spine/phases/04-plan.md`).
SELECT
    repo_id,
    event_date,
    COUNT(*) AS event_count,
    COUNT(DISTINCT actor_id) AS distinct_actors
FROM smelt.silver.events_deduped
GROUP BY repo_id, event_date
