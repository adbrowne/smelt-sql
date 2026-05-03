---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    device_type,
    SUM(revenue) AS agg_0,
    AVG(price) AS agg_1
FROM smelt.sql_l1_173
GROUP BY device_type

