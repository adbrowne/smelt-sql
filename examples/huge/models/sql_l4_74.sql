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
    event_time,
    category
FROM smelt.models.sql_l3_193
WHERE user_id IN (
    SELECT user_id FROM smelt.models.sql_l3_193 WHERE platform = 'web'
)

