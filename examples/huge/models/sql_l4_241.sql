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
    SUM(quantity) AS metric_1,
    AVG(duration_seconds) AS metric_2
FROM smelt.models.sql_l3_33
GROUP BY DATE_TRUNC('week', event_time)

