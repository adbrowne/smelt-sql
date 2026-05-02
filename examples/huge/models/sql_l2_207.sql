---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.transaction_id,
    b.is_verified,
    c.rating,
    c.ip_address
FROM smelt.models.sql_l1_36 a
INNER JOIN smelt.models.sql_l1_130 b ON a.user_id = b.user_id
LEFT JOIN smelt.models.sql_l1_13 c ON a.user_id = c.user_id

