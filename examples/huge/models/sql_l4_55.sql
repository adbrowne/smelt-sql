---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    os_name,
    device_type,
    ROW_NUMBER() OVER (PARTITION BY os_name ORDER BY created_at) AS win_val
FROM smelt.ref('py_l3_328')
