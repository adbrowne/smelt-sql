---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    cost,
    SUM(revenue) AS agg_0,
    COUNT(*) AS agg_1,
    SUM(quantity) AS agg_2
FROM smelt.categories
GROUP BY cost
