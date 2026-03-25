---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    DATE_TRUNC('week', event_time) AS period,
    SUM(quantity) AS metric_1,
    AVG(duration_seconds) AS metric_2
FROM smelt.ref('sql_l3_66')
GROUP BY DATE_TRUNC('week', event_time)
