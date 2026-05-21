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
    profit,
    country,
    tier
FROM smelt.sql_l1_209
WHERE user_id IN (
    SELECT user_id FROM smelt.sql_l1_114 WHERE score >= 50
)

