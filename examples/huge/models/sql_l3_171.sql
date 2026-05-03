---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    is_active,
    discount,
    product_id
FROM smelt.sql_l2_241
WHERE user_id IN (
    SELECT user_id FROM smelt.sql_l2_137 WHERE amount > 0
)

