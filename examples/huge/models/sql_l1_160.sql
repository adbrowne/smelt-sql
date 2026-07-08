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
    DATE_TRUNC('week', event_time) AS period,
    COUNT(DISTINCT user_id) AS metric_1,
    MIN(created_at) AS metric_2
FROM smelt.products
GROUP BY DATE_TRUNC('week', event_time)
