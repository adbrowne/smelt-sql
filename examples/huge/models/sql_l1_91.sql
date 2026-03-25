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
    a.created_at,
    b.amount
FROM smelt.ref('signups') a
INNER JOIN smelt.ref('signups') b ON a.user_id = b.user_id
