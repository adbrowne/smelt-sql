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
    session_id,
    category,
    transaction_id
FROM smelt.events
WHERE score >= 50
