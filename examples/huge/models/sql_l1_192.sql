---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    channel,
    country,
    browser
FROM smelt.signups
WHERE user_id IN (
    SELECT user_id FROM smelt.signups WHERE event_type = 'purchase'
)
