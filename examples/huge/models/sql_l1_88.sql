---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.event_time,
    a.tier,
    b.page_path
FROM smelt.ref('subscriptions') a
INNER JOIN smelt.ref('subscriptions') b ON a.user_id = b.user_id
