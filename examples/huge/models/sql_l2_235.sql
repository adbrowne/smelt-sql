---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    event_time,
    score,
    region,
    segment
FROM smelt.ref('py_l1_447')
WHERE is_active = true
