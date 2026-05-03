---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.duration_seconds,
    a.referrer,
    b.page_path
FROM smelt.clicks a
LEFT JOIN smelt.clicks b ON a.user_id = b.user_id

