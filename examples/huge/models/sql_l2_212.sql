---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    category,
    AVG(amount) AS agg_0,
    SUM(quantity) AS agg_1,
    MIN(created_at) AS agg_2,
    SUM(amount) AS agg_3,
    MAX(created_at) AS agg_4
FROM smelt.sql_l1_25
GROUP BY category
