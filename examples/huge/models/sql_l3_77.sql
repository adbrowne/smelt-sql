---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    DATE_TRUNC('week', event_time) AS period,
    MIN(created_at) AS metric_1,
    COUNT(*) AS metric_2
FROM smelt.sql_l2_247
GROUP BY DATE_TRUNC('week', event_time)

