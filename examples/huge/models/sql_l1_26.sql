---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    platform,
    ip_address,
    product_id,
    country
FROM smelt.models.campaigns
WHERE event_type = 'purchase'

