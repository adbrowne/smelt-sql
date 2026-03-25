---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    is_verified,
    channel,
    ROW_NUMBER() OVER (PARTITION BY is_verified ORDER BY created_at) AS win_val
FROM smelt.ref('py_l1_362')
