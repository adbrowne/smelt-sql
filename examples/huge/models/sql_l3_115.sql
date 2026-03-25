---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    email_domain,
    price,
    profit
FROM smelt.ref('sql_l2_101')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('sql_l2_93') WHERE country = 'US'
)
