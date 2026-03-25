---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.profit,
    b.page_path,
    c.device_type,
    c.cost
FROM smelt.ref('py_l2_392') a
INNER JOIN smelt.ref('py_l2_466') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('py_l2_392') c ON a.user_id = c.user_id
