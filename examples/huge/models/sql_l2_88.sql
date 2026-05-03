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
    SUM(amount) AS metric_1,
    SUM(quantity) AS metric_2
FROM smelt.sql_l1_31
GROUP BY DATE_TRUNC('month', event_time)

