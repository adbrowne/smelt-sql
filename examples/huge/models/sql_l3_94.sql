---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    DATE_TRUNC('day', event_time) AS period,
    MAX(created_at) AS metric_1,
    MIN(created_at) AS metric_2
FROM smelt.ref('sql_l2_224')
GROUP BY DATE_TRUNC('day', event_time)
