---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    email_domain,
    amount,
    is_verified
FROM smelt.ref('sql_l2_234')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('sql_l2_224') WHERE platform = 'web'
)
