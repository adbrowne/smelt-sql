---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    campaign_id,
    price,
    channel
FROM smelt.models.sql_l2_0
WHERE user_id IN (
    SELECT user_id FROM smelt.models.sql_l2_83 WHERE quantity > 0
)

