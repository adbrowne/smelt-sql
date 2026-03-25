---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.updated_at,
    a.event_type,
    b.quantity
FROM smelt.ref('shipments') a
LEFT JOIN smelt.ref('shipments') b ON a.user_id = b.user_id
