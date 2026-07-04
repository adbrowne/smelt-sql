---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    channel,
    email_domain,
    segment,
    is_active
FROM smelt.sql_l2_118
WHERE status = 'active'
