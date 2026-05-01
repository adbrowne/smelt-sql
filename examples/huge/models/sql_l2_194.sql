---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    DATE_TRUNC('day', event_time) AS period,
    AVG(price) AS metric_1,
    MIN(created_at) AS metric_2
FROM smelt.models.sql_l1_82
GROUP BY DATE_TRUNC('day', event_time)

