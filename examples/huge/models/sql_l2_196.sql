---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    category,
    amount,
    ROW_NUMBER() OVER (PARTITION BY category ORDER BY created_at) AS win_val
FROM smelt.sql_l1_126
