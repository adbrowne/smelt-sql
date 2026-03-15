---
materialized: table
incremental:
  partition_column: event_date
  event_time_column: event_timestamp
  granularity: day
---
-- Incremental materialization: only process new partitions
-- Run with: smelt run --select daily_events --event-time-start 2024-01-01 --event-time-end 2024-01-06
SELECT
    date_trunc('day', event_timestamp) as event_date,
    user_id,
    COUNT(*) as event_count
FROM smelt.source('raw.events')
GROUP BY 1, 2
