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
    amount,
    SUM(amount) AS agg_0,
    SUM(revenue) AS agg_1,
    SUM(quantity) AS agg_2,
    COUNT(*) AS agg_3,
    AVG(price) AS agg_4
FROM smelt.errors
GROUP BY amount

