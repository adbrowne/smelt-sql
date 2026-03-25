---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    device_type,
    cost,
    RANK() OVER (PARTITION BY device_type ORDER BY created_at) AS win_val
FROM smelt.ref('py_l3_454')
