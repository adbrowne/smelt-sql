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
    score,
    AVG(duration_seconds) AS agg_0,
    SUM(quantity) AS agg_1,
    MAX(created_at) AS agg_2
FROM smelt.sql_l2_50
GROUP BY score

