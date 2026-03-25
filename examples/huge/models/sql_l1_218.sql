---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.platform,
    a.event_type,
    b.quantity
FROM smelt.ref('logs') a
INNER JOIN smelt.ref('logs') b ON a.user_id = b.user_id
