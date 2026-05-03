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
    AVG(duration_seconds) AS metric_1,
    SUM(amount) AS metric_2
FROM smelt.sql_l1_148
GROUP BY DATE_TRUNC('day', event_time)

