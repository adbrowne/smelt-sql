---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    platform,
    ip_address,
    product_id,
    country
FROM smelt.ref('campaigns')
WHERE event_type = 'purchase'
