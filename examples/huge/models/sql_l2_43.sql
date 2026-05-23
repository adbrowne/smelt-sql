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
    status,
    cost,
    product_id
FROM smelt.sql_l1_26
WHERE user_id IN (
    SELECT user_id FROM smelt.sql_l1_26 WHERE event_type = 'purchase'
)
