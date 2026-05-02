---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    transaction_id,
    event_time,
    created_at
FROM smelt.models.shipments
WHERE user_id IN (
    SELECT user_id FROM smelt.models.shipments WHERE category IS NOT NULL
)

