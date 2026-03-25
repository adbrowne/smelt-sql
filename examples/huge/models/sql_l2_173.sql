---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.amount,
    b.region,
    c.revenue,
    c.event_type
FROM smelt.ref('py_l1_467') a
INNER JOIN smelt.ref('sql_l1_13') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('sql_l1_26') c ON a.user_id = c.user_id
