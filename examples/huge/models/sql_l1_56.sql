---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.revenue,
    b.price,
    c.country,
    c.page_path
FROM smelt.ref('users') a
INNER JOIN smelt.ref('users') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('users') c ON a.user_id = c.user_id
