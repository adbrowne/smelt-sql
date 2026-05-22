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
    order_id,
    AVG(amount) AS agg_0,
    SUM(revenue) AS agg_1,
    SUM(amount) AS agg_2
FROM smelt.sql_l2_55
GROUP BY order_id
