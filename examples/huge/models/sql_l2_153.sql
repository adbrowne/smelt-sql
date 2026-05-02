---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    session_id,
    email_domain,
    platform
FROM smelt.models.sql_l1_190
WHERE user_id IN (
    SELECT user_id FROM smelt.models.sql_l1_190 WHERE is_active = true
)

