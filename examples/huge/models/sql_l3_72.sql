---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    campaign_id,
    price,
    channel
FROM smelt.sql_l2_0
WHERE user_id IN (
    SELECT user_id FROM smelt.sql_l2_83 WHERE quantity > 0
)
