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
    COUNT(DISTINCT user_id) AS metric_2
FROM smelt.ref('sql_l2_174')
GROUP BY DATE_TRUNC('month', event_time)
