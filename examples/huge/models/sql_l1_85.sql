---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.channel,
    a.campaign_id,
    b.cost
FROM smelt.models.reviews a
LEFT JOIN smelt.models.reviews b ON a.user_id = b.user_id

