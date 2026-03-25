---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.category,
    a.page_path,
    b.profit
FROM smelt.ref('py_l1_256') a
LEFT JOIN smelt.ref('py_l1_439') b ON a.user_id = b.user_id
