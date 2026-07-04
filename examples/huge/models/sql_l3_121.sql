---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    cost,
    email_domain,
    amount
FROM smelt.sql_l2_177
WHERE user_id IN (
    SELECT user_id FROM smelt.sql_l2_153 WHERE event_type = 'purchase'
)
