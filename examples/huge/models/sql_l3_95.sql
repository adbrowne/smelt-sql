---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.created_at,
    b.revenue,
    c.rating,
    c.duration_seconds
FROM smelt.ref('sql_l2_98') a
INNER JOIN smelt.ref('sql_l2_197') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('sql_l2_98') c ON a.user_id = c.user_id
