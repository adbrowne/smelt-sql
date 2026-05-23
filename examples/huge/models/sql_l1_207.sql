---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    transaction_id,
    event_time,
    created_at
FROM smelt.shipments
WHERE user_id IN (
    SELECT user_id FROM smelt.shipments WHERE category IS NOT NULL
)
