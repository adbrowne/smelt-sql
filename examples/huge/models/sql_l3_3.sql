---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    price,
    is_verified,
    platform
FROM smelt.ref('sql_l2_184')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('sql_l2_29') WHERE quantity > 0
)
