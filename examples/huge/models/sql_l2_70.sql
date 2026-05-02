---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    order_id,
    user_id,
    os_name
FROM smelt.models.sql_l1_62
WHERE user_id IN (
    SELECT user_id FROM smelt.models.sql_l1_62 WHERE created_at >= '2024-01-01'
)

