---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.is_active,
    a.order_id,
    b.product_id
FROM smelt.ref('sql_l1_11') a
INNER JOIN smelt.ref('sql_l1_11') b ON a.user_id = b.user_id
