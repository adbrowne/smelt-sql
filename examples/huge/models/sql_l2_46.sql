---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    DATE_TRUNC('day', event_time) AS period,
    AVG(price) AS metric_1,
    SUM(quantity) AS metric_2
FROM smelt.sql_l1_25
GROUP BY DATE_TRUNC('day', event_time)
