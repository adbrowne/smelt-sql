---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.user_id,
    b.revenue,
    c.quantity,
    c.rating
FROM smelt.ref('sql_l3_32') a
INNER JOIN smelt.ref('sql_l3_57') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('sql_l3_32') c ON a.user_id = c.user_id
