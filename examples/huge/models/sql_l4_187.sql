---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.event_time,
    a.score,
    b.order_id
FROM smelt.ref('sql_l3_150') a
INNER JOIN smelt.ref('py_l3_253') b ON a.user_id = b.user_id
