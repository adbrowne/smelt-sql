---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.product_id,
    a.duration_seconds,
    b.referrer
FROM smelt.ref('py_l2_377') a
INNER JOIN smelt.ref('sql_l2_245') b ON a.user_id = b.user_id
