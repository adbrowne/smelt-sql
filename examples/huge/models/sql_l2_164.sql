---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    plan_type,
    SUM(revenue) AS agg_0,
    AVG(amount) AS agg_1
FROM smelt.sql_l1_233
GROUP BY plan_type
