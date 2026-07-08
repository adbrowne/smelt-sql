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
    status,
    ip_address,
    device_type,
    order_id
FROM smelt.users
WHERE status = 'active'
