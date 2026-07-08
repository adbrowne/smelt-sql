---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    country,
    updated_at,
    campaign_id
FROM smelt.payments
WHERE user_id IN (
    SELECT user_id FROM smelt.payments WHERE country = 'US'
)
