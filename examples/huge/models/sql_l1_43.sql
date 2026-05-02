---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    referrer,
    email_domain,
    duration_seconds,
    category
FROM smelt.models.categories
WHERE quantity > 0

