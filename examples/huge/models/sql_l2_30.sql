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
    score,
    profit,
    segment,
    tier
FROM smelt.sql_l1_96
WHERE platform = 'web'
