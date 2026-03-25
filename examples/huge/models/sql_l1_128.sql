---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    event_date,
    product_id,
    referrer
FROM smelt.ref('clicks')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('clicks') WHERE status = 'active'
)
