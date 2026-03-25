---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    status,
    referrer,
    duration_seconds
FROM smelt.ref('reviews')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('reviews') WHERE event_type = 'purchase'
)
