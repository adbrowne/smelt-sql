---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    email_domain,
    quantity,
    is_active,
    amount
FROM smelt.errors
WHERE event_type = 'purchase'
