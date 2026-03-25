---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.page_path,
    b.email_domain,
    c.device_type,
    c.category
FROM smelt.ref('sql_l2_8') a
INNER JOIN smelt.ref('py_l2_438') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('py_l2_426') c ON a.user_id = c.user_id
