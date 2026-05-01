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
    AVG(amount) AS metric_2
FROM smelt.models.sql_l1_156
GROUP BY DATE_TRUNC('week', event_time)

