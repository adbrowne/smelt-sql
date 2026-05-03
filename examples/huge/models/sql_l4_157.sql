---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    score,
    device_type,
    quantity,
    event_time
FROM smelt.sql_l3_5
WHERE quantity > 0

