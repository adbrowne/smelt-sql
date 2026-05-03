---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    segment,
    transaction_id,
    ROW_NUMBER() OVER (PARTITION BY segment ORDER BY created_at) AS win_val
FROM smelt.orders

