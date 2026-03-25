---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.session_id,
    a.page_path,
    b.updated_at
FROM smelt.ref('sql_l1_86') a
INNER JOIN smelt.ref('py_l1_356') b ON a.user_id = b.user_id
