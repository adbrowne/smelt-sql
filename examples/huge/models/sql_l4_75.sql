---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    platform,
    SUM(amount) AS agg_0,
    AVG(amount) AS agg_1,
    COUNT(*) AS agg_2
FROM smelt.sql_l3_108
GROUP BY platform

