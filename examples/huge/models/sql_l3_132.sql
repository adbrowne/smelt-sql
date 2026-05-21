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
    is_active,
    SUM(revenue) AS agg_0,
    MIN(created_at) AS agg_1,
    AVG(duration_seconds) AS agg_2,
    SUM(quantity) AS agg_3,
    AVG(amount) AS agg_4
FROM smelt.sql_l2_226
GROUP BY is_active

