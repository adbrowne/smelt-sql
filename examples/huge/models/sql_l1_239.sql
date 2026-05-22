---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    status,
    platform,
    is_verified
FROM smelt.page_views
WHERE user_id IN (
    SELECT user_id FROM smelt.page_views WHERE score >= 50
)
