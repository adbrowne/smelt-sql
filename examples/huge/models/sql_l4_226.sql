---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.discount,
    b.email_domain,
    c.price,
    c.device_type
FROM smelt.ref('sql_l3_19') a
INNER JOIN smelt.ref('py_l3_306') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('sql_l3_246') c ON a.user_id = c.user_id
