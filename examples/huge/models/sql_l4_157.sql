---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    score,
    device_type,
    quantity,
    event_time
FROM smelt.ref('sql_l3_126')
WHERE quantity > 0
