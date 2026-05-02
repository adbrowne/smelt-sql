---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.user_id,
    a.quantity,
    b.product_id
FROM smelt.models.sql_l2_92 a
INNER JOIN smelt.models.sql_l2_70 b ON a.user_id = b.user_id

