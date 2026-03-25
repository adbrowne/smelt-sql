---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.email_domain,
    b.device_type,
    c.user_id,
    c.is_active
FROM smelt.ref('sql_l1_130') a
INNER JOIN smelt.ref('py_l1_419') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('sql_l1_13') c ON a.user_id = c.user_id
