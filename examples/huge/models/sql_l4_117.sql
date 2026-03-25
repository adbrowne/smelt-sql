---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    duration_seconds,
    price,
    RANK() OVER (PARTITION BY duration_seconds ORDER BY created_at) AS win_val
FROM smelt.ref('py_l3_458')
