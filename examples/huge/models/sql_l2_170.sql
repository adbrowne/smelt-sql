---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    duration_seconds,
    email_domain,
    event_type,
    price
FROM smelt.ref('sql_l1_68')
WHERE status = 'active'
