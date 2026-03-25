---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    event_date,
    product_id,
    referrer
FROM smelt.ref('clicks')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('clicks') WHERE status = 'active'
)
