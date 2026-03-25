---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    channel,
    email_domain,
    segment,
    is_active
FROM smelt.ref('sql_l2_200')
WHERE status = 'active'
