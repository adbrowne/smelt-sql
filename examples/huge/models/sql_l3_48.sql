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
    COUNT(DISTINCT user_id) AS metric_2
FROM smelt.ref('sql_l2_43')
GROUP BY DATE_TRUNC('week', event_time)
