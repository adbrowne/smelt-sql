---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    score,
    quantity,
    ROW_NUMBER() OVER (PARTITION BY score ORDER BY created_at) AS win_val
FROM smelt.ref('py_l3_435')
