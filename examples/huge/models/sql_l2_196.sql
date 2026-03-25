---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    category,
    amount,
    ROW_NUMBER() OVER (PARTITION BY category ORDER BY created_at) AS win_val
FROM smelt.ref('py_l1_429')
