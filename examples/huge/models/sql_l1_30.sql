---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.device_type,
    a.created_at,
    b.discount
FROM smelt.ref('subscriptions') a
LEFT JOIN smelt.ref('subscriptions') b ON a.user_id = b.user_id
