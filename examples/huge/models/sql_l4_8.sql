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
    AVG(price) AS agg_0,
    AVG(amount) AS agg_1
FROM smelt.sql_l3_127
GROUP BY order_id

