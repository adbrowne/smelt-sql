---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    DATE_TRUNC('week', event_time) AS period,
    MIN(created_at) AS metric_1,
    COUNT(DISTINCT user_id) AS metric_2
FROM smelt.ref('sql_l1_190')
GROUP BY DATE_TRUNC('week', event_time)
