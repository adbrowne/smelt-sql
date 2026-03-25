---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.referrer,
    b.created_at,
    c.transaction_id,
    c.category
FROM smelt.ref('sql_l1_102') a
INNER JOIN smelt.ref('py_l1_287') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('sql_l1_102') c ON a.user_id = c.user_id
