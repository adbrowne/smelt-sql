---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    status,
    AVG(price) AS agg_0,
    COUNT(*) AS agg_1,
    AVG(amount) AS agg_2
FROM smelt.categories
GROUP BY status

