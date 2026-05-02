---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    is_verified,
    discount,
    email_domain,
    referrer
FROM smelt.models.categories
WHERE platform = 'web'

