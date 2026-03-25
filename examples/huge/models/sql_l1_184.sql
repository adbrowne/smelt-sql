---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.browser,
    b.transaction_id,
    c.updated_at,
    c.revenue
FROM smelt.ref('sessions') a
INNER JOIN smelt.ref('sessions') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('sessions') c ON a.user_id = c.user_id
