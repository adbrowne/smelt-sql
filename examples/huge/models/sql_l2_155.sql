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
    is_verified,
    COUNT(DISTINCT user_id) AS agg_0,
    COUNT(*) AS agg_1,
    AVG(duration_seconds) AS agg_2,
    SUM(revenue) AS agg_3
FROM smelt.sql_l1_164
GROUP BY is_verified
