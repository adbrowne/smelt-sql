---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    email_domain,
    quantity,
    is_active,
    amount
FROM smelt.ref('errors')
WHERE event_type = 'purchase'
