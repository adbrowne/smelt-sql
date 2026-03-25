---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    referrer,
    event_date,
    profit
FROM smelt.ref('clicks')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('clicks') WHERE status = 'active'
)
