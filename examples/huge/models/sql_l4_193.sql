---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    DATE_TRUNC('month', event_time) AS period,
    MAX(created_at) AS metric_1,
    COUNT(*) AS metric_2
FROM smelt.ref('sql_l3_244')
GROUP BY DATE_TRUNC('month', event_time)
