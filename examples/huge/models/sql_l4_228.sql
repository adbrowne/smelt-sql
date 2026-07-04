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
    AVG(amount) AS agg_0,
    SUM(quantity) AS agg_1,
    SUM(amount) AS agg_2
FROM smelt.sql_l3_223
GROUP BY amount
