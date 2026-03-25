---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    email_domain,
    amount,
    is_verified
FROM smelt.ref('sql_l2_0')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('sql_l2_29') WHERE platform = 'web'
)
