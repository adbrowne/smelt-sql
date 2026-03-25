---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    cost,
    user_id,
    RANK() OVER (PARTITION BY cost ORDER BY created_at) AS win_val
FROM smelt.ref('py_l2_472')
