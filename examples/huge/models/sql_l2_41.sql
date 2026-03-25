---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.discount,
    b.session_id,
    c.ip_address,
    c.quantity
FROM smelt.ref('sql_l1_233') a
INNER JOIN smelt.ref('sql_l1_233') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('sql_l1_233') c ON a.user_id = c.user_id
