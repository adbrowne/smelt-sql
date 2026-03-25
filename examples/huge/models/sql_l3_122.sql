---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.revenue,
    b.channel,
    c.segment,
    c.transaction_id
FROM smelt.ref('py_l2_300') a
INNER JOIN smelt.ref('sql_l2_130') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('sql_l2_118') c ON a.user_id = c.user_id
