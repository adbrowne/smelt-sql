---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.price,
    b.device_type,
    c.amount,
    c.created_at
FROM smelt.ref('sql_l1_49') a
INNER JOIN smelt.ref('sql_l1_49') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('sql_l1_49') c ON a.user_id = c.user_id
