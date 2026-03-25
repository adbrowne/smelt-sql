---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.revenue,
    a.category,
    b.is_active
FROM smelt.ref('sql_l2_163') a
INNER JOIN smelt.ref('py_l2_433') b ON a.user_id = b.user_id
