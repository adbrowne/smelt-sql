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
FROM smelt.reviews a
LEFT JOIN smelt.reviews b ON a.user_id = b.user_id

