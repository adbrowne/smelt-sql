---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    channel,
    country,
    browser
FROM smelt.models.signups
WHERE user_id IN (
    SELECT user_id FROM smelt.models.signups WHERE event_type = 'purchase'
)

