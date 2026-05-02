---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    os_name,
    transaction_id,
    email_domain
FROM smelt.models.sql_l2_109
WHERE user_id IN (
    SELECT user_id FROM smelt.models.sql_l2_114 WHERE created_at >= '2024-01-01'
)

