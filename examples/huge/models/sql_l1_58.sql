---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    campaign_id,
    platform,
    channel,
    transaction_id
FROM smelt.payments
WHERE platform = 'web'
