---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    country,
    region,
    campaign_id,
    updated_at
FROM smelt.ref('clicks')
WHERE status = 'active'
