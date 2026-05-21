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
    rating,
    SUM(quantity) AS agg_0,
    AVG(amount) AS agg_1,
    COUNT(DISTINCT user_id) AS agg_2,
    SUM(amount) AS agg_3
FROM smelt.sql_l3_178
GROUP BY rating

