---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    platform,
    ip_address,
    product_id,
    country
FROM smelt.campaigns
WHERE event_type = 'purchase'
