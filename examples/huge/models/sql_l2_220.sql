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
    score,
    email_domain,
    discount
FROM smelt.sql_l1_67
WHERE user_id IN (
    SELECT user_id FROM smelt.sql_l1_56 WHERE status = 'active'
)

