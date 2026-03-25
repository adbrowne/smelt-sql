---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    status,
    referrer,
    duration_seconds
FROM smelt.ref('reviews')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('reviews') WHERE event_type = 'purchase'
)
