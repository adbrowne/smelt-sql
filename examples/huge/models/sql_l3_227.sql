---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    price,
    device_type,
    RANK() OVER (PARTITION BY price ORDER BY created_at) AS win_val
FROM smelt.ref('py_l2_361')
