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
    COUNT(DISTINCT user_id) AS metric_1,
    AVG(price) AS metric_2
FROM smelt.models.sql_l2_40
GROUP BY DATE_TRUNC('month', event_time)

