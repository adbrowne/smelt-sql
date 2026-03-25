---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    event_date,
    created_at,
    RANK() OVER (PARTITION BY event_date ORDER BY created_at) AS win_val
FROM smelt.ref('py_l1_251')
