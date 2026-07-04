---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    device_type,
    AVG(duration_seconds) AS agg_0,
    SUM(quantity) AS agg_1,
    MAX(created_at) AS agg_2,
    AVG(price) AS agg_3
FROM smelt.sql_l2_8
GROUP BY device_type
