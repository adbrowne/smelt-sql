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
    plan_type,
    AVG(amount) AS agg_0,
    COUNT(DISTINCT user_id) AS agg_1,
    SUM(quantity) AS agg_2,
    MIN(created_at) AS agg_3
FROM smelt.sql_l1_33
GROUP BY plan_type

