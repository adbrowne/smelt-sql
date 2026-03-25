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
    SUM(revenue) AS metric_2
FROM smelt.ref('sql_l3_139')
GROUP BY DATE_TRUNC('day', event_time)
