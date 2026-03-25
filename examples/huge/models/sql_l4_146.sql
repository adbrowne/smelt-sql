---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.order_id,
    b.event_date,
    c.segment,
    c.status
FROM smelt.ref('py_l3_486') a
INNER JOIN smelt.ref('py_l3_486') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('py_l3_486') c ON a.user_id = c.user_id
