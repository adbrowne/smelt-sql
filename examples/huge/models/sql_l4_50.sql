---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    region,
    browser,
    revenue,
    event_date
FROM smelt.sql_l3_217
WHERE country = 'US'
