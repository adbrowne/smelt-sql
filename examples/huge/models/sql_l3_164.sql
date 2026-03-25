---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.os_name,
    b.profit,
    c.user_id,
    c.page_path
FROM smelt.ref('py_l2_477') a
INNER JOIN smelt.ref('py_l2_322') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('py_l2_416') c ON a.user_id = c.user_id
