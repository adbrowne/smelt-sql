---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.revenue,
    a.quantity,
    b.is_verified
FROM smelt.ref('refunds') a
LEFT JOIN smelt.ref('refunds') b ON a.user_id = b.user_id
