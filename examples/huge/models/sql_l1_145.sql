---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.score,
    a.plan_type,
    b.region
FROM smelt.ref('events') a
LEFT JOIN smelt.ref('events') b ON a.user_id = b.user_id
