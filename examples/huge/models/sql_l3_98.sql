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
    user_id,
    is_verified,
    cost,
    region
FROM smelt.sql_l2_85
WHERE quantity > 0
