---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    score,
    email_domain,
    discount
FROM smelt.models.sql_l1_67
WHERE user_id IN (
    SELECT user_id FROM smelt.models.sql_l1_56 WHERE status = 'active'
)

