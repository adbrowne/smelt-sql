---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    device_type,
    tier,
    campaign_id
FROM smelt.models.sql_l2_136
WHERE user_id IN (
    SELECT user_id FROM smelt.models.sql_l2_102 WHERE country = 'US'
)

