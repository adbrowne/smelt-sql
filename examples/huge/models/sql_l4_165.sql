---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.ip_address,
    b.cost,
    c.updated_at,
    c.event_date
FROM smelt.ref('py_l3_283') a
INNER JOIN smelt.ref('sql_l3_179') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('py_l3_372') c ON a.user_id = c.user_id
