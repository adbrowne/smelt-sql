---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    referrer,
    event_date,
    profit
FROM smelt.clicks
WHERE user_id IN (
    SELECT user_id FROM smelt.clicks WHERE status = 'active'
)
