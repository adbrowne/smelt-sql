---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.price,
    a.cost,
    b.category
FROM smelt.ref('py_l1_398') a
INNER JOIN smelt.ref('py_l1_398') b ON a.user_id = b.user_id
