---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    DATE_TRUNC('day', event_time) AS period,
    COUNT(DISTINCT user_id) AS metric_1,
    AVG(duration_seconds) AS metric_2
FROM smelt.ref('sql_l1_11')
GROUP BY DATE_TRUNC('day', event_time)
