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
    COUNT(*) AS metric_1,
    COUNT(DISTINCT user_id) AS metric_2
FROM smelt.sql_l3_94
GROUP BY DATE_TRUNC('day', event_time)

