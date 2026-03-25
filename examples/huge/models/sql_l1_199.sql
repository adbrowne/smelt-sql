---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.is_active,
    a.event_type,
    b.status
FROM smelt.ref('payments') a
LEFT JOIN smelt.ref('payments') b ON a.user_id = b.user_id
