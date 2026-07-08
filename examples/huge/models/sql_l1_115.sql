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
    a.os_name,
    b.discount,
    c.segment,
    c.quantity
FROM smelt.events a
INNER JOIN smelt.events b ON a.user_id = b.user_id
LEFT JOIN smelt.events c ON a.user_id = c.user_id
