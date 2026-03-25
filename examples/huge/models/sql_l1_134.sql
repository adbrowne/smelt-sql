---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    duration_seconds,
    email_domain,
    event_date,
    channel
FROM smelt.ref('categories')
WHERE category IS NOT NULL
