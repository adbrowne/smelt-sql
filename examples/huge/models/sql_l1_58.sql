---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    campaign_id,
    platform,
    channel,
    transaction_id
FROM smelt.ref('payments')
WHERE platform = 'web'
