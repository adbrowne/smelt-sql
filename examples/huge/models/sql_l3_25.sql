---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.transaction_id,
    b.profit,
    c.status,
    c.discount
FROM smelt.ref('py_l2_281') a
INNER JOIN smelt.ref('sql_l2_137') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('py_l2_309') c ON a.user_id = c.user_id
