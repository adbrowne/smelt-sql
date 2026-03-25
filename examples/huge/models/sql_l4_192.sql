---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    created_at,
    revenue,
    RANK() OVER (PARTITION BY created_at ORDER BY created_at) AS win_val
FROM smelt.ref('py_l3_279')
