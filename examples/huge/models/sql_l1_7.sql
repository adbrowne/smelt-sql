---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    cost,
    browser,
    region,
    status
FROM smelt.events
WHERE event_type = 'purchase'
