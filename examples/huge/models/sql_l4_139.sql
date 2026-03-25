---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.status,
    a.page_path,
    b.is_verified
FROM smelt.ref('py_l3_452') a
INNER JOIN smelt.ref('sql_l3_15') b ON a.user_id = b.user_id
