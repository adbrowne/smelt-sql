---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    score,
    profit,
    segment,
    tier
FROM smelt.models.sql_l1_96
WHERE platform = 'web'

