---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    DATE_TRUNC('day', event_time) AS period,
    COUNT(DISTINCT user_id) AS metric_1,
    AVG(duration_seconds) AS metric_2
FROM smelt.sql_l1_123
GROUP BY DATE_TRUNC('day', event_time)
