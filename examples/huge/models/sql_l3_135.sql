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
    os_name,
    transaction_id,
    email_domain
FROM smelt.sql_l2_109
WHERE user_id IN (
    SELECT user_id FROM smelt.sql_l2_114 WHERE created_at >= '2024-01-01'
)

