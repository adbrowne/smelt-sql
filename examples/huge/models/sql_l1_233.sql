---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    event_type,
    platform,
    transaction_id,
    quantity
FROM smelt.models.products
WHERE score >= 50

