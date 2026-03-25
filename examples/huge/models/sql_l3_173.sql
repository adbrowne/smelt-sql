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
    MAX(created_at) AS metric_1,
    AVG(amount) AS metric_2
FROM smelt.ref('sql_l2_228')
GROUP BY DATE_TRUNC('week', event_time)
