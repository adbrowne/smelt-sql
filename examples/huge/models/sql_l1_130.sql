---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    transaction_id,
    score,
    tier,
    quantity
FROM smelt.ref('clicks')
WHERE platform = 'web'
