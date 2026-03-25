---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    session_id,
    email_domain,
    platform
FROM smelt.ref('sql_l1_138')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('sql_l1_138') WHERE is_active = true
)
