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
    cost,
    discount
FROM smelt.sql_l1_189
WHERE user_id IN (
    SELECT user_id FROM smelt.sql_l1_14 WHERE status = 'active'
)
