---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    discount,
    browser,
    user_id
FROM smelt.orders
WHERE user_id IN (
    SELECT user_id FROM smelt.orders WHERE country = 'US'
)
