---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    ip_address,
    event_date,
    region
FROM smelt.transactions
WHERE user_id IN (
    SELECT user_id FROM smelt.transactions WHERE status = 'active'
)
