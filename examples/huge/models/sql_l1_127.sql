---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    amount,
    category,
    event_time,
    profit
FROM smelt.categories
WHERE amount > 0
