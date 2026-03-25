---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    is_verified,
    discount,
    email_domain,
    referrer
FROM smelt.ref('categories')
WHERE platform = 'web'
