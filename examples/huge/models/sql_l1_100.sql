---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    is_verified,
    discount,
    email_domain,
    referrer
FROM smelt.categories
WHERE platform = 'web'
