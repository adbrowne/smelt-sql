---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    status,
    cost,
    product_id
FROM smelt.models.sql_l1_26
WHERE user_id IN (
    SELECT user_id FROM smelt.models.sql_l1_26 WHERE event_type = 'purchase'
)

