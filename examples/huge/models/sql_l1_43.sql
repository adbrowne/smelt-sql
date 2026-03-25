---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    referrer,
    email_domain,
    duration_seconds,
    category
FROM smelt.ref('categories')
WHERE quantity > 0
