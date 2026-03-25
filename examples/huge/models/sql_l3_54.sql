---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.revenue,
    b.user_id,
    c.browser,
    c.quantity
FROM smelt.ref('py_l2_425') a
INNER JOIN smelt.ref('sql_l2_233') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('py_l2_425') c ON a.user_id = c.user_id
