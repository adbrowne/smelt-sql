---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    category,
    campaign_id,
    country,
    region
FROM smelt.ref('sessions')
WHERE event_type = 'purchase'
