---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    category,
    campaign_id,
    country,
    region
FROM smelt.sessions
WHERE event_type = 'purchase'

