---
materialization: table
refresh: incremental
grain: partition
timeseries:
  partition_column: event_date
  event_time_column: event_timestamp
  granularity: day
---
-- Composed: cube split + incremental materialization
-- The planner detects both annotations and composes them:
-- 1. Cube split: parallel sub-queries for each COUNT(DISTINCT)
-- 2. Incremental: time-filtered execution with DELETE+INSERT
-- Run with: smelt run --select daily_cube_metrics --event-time-start 2024-01-01 --event-time-end 2024-01-06
SELECT
    date_trunc('day', event_timestamp) as event_date,
    event_type,
    COUNT(DISTINCT user_id) as unique_users,
    COUNT(DISTINCT event_id) as unique_events,
    COUNT(*) as total_events
FROM smelt.sources.raw.events
GROUP BY 1, 2 -- smelt:cube_split

