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
    price,
    profit
FROM smelt.ref('sql_l2_185')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('sql_l2_248') WHERE country = 'US'
)
