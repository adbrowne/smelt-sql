---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    plan_type,
    duration_seconds,
    profit,
    updated_at
FROM smelt.sql_l1_178
WHERE category IS NOT NULL
