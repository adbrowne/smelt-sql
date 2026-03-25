---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.product_id,
    b.email_domain,
    c.os_name,
    c.duration_seconds
FROM smelt.ref('sql_l3_19') a
INNER JOIN smelt.ref('py_l3_298') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('sql_l3_19') c ON a.user_id = c.user_id
