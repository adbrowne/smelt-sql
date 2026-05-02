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
FROM smelt.models.clicks a
LEFT JOIN smelt.models.clicks b ON a.user_id = b.user_id

