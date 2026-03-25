---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.page_path,
    a.user_id,
    b.session_id
FROM smelt.ref('sql_l3_109') a
LEFT JOIN smelt.ref('py_l3_318') b ON a.user_id = b.user_id
