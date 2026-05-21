---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    plan_type,
    country,
    profit,
    category
FROM smelt.sql_l2_102
WHERE quantity > 0

