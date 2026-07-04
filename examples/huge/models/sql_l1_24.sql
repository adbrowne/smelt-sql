---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.event_type,
    b.referrer,
    c.os_name,
    c.cost
FROM smelt.payments a
INNER JOIN smelt.payments b ON a.user_id = b.user_id
LEFT JOIN smelt.payments c ON a.user_id = c.user_id
