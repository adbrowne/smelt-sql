---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.profit,
    b.cost,
    c.revenue,
    c.category
FROM smelt.ref('py_l2_338') a
INNER JOIN smelt.ref('py_l2_458') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('py_l2_471') c ON a.user_id = c.user_id
