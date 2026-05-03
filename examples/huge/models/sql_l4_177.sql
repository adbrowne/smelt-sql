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
    SUM(revenue) AS metric_1,
    MIN(created_at) AS metric_2
FROM smelt.sql_l3_46
GROUP BY DATE_TRUNC('week', event_time)

