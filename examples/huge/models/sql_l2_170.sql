---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    duration_seconds,
    email_domain,
    event_type,
    price
FROM smelt.sql_l1_201
WHERE status = 'active'

