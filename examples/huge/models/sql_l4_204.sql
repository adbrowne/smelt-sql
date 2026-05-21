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
    channel,
    quantity,
    profit
FROM smelt.sql_l3_190
WHERE user_id IN (
    SELECT user_id FROM smelt.sql_l3_103 WHERE score >= 50
)

