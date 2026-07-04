---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.profit,
    a.device_type,
    b.status
FROM smelt.logs a
INNER JOIN smelt.logs b ON a.user_id = b.user_id
