---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.order_id,
    b.browser,
    c.plan_type,
    c.cohort_date
FROM smelt.sql_l1_161 a
INNER JOIN smelt.sql_l1_161 b ON a.user_id = b.user_id
LEFT JOIN smelt.sql_l1_161 c ON a.user_id = c.user_id
