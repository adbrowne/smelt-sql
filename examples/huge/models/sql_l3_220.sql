---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.duration_seconds,
    a.product_id,
    b.status
FROM smelt.ref('sql_l2_151') a
INNER JOIN smelt.ref('sql_l2_197') b ON a.user_id = b.user_id
