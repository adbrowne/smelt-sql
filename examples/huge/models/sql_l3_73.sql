---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    DATE_TRUNC('month', event_time) AS period,
    AVG(duration_seconds) AS metric_1,
    SUM(quantity) AS metric_2
FROM smelt.models.sql_l2_213
GROUP BY DATE_TRUNC('month', event_time)

