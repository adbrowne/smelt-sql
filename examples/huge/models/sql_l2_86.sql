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
    amount,
    status,
    discount
FROM smelt.sql_l1_191
WHERE amount > 0
