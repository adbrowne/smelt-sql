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
    AVG(amount) AS metric_1,
    COUNT(DISTINCT user_id) AS metric_2
FROM smelt.subscriptions
GROUP BY DATE_TRUNC('week', event_time)

