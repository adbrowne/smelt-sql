---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    event_time,
    platform,
    RANK() OVER (PARTITION BY event_time ORDER BY created_at) AS win_val
FROM smelt.ref('py_l1_331')
