---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    transaction_id,
    event_time,
    created_at
FROM smelt.ref('shipments')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('shipments') WHERE category IS NOT NULL
)
