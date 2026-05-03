---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    country,
    region,
    campaign_id,
    updated_at
FROM smelt.clicks
WHERE status = 'active'

