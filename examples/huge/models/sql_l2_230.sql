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
    created_at,
    quantity,
    product_id
FROM smelt.sql_l1_12
WHERE user_id IN (
    SELECT user_id FROM smelt.sql_l1_12 WHERE score >= 50
)

