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
    quantity,
    is_active,
    amount
FROM smelt.ref('errors')
WHERE event_type = 'purchase'
