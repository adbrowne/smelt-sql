---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.platform,
    a.event_type,
    b.quantity
FROM smelt.ref('logs') a
INNER JOIN smelt.ref('logs') b ON a.user_id = b.user_id
