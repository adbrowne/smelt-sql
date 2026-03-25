---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.discount,
    a.revenue,
    b.rating
FROM smelt.ref('sql_l3_2') a
INNER JOIN smelt.ref('py_l3_386') b ON a.user_id = b.user_id
