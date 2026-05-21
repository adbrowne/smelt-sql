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
    user_id,
    COUNT(DISTINCT user_id) AS agg_0,
    AVG(duration_seconds) AS agg_1,
    SUM(quantity) AS agg_2,
    SUM(revenue) AS agg_3
FROM smelt.sql_l3_202
GROUP BY user_id

