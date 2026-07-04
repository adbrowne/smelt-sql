---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    browser,
    campaign_id,
    session_id
FROM smelt.sql_l1_17
WHERE user_id IN (
    SELECT user_id FROM smelt.sql_l1_17 WHERE country = 'US'
)
