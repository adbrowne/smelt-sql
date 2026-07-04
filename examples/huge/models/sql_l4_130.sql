---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    page_path,
    email_domain,
    event_type
FROM smelt.sql_l3_36
WHERE user_id IN (
    SELECT user_id FROM smelt.sql_l3_8 WHERE status = 'active'
)
