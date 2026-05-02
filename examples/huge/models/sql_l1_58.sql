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
    platform,
    channel,
    transaction_id
FROM smelt.models.payments
WHERE platform = 'web'

