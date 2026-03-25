---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    is_active,
    updated_at,
    ROW_NUMBER() OVER (PARTITION BY is_active ORDER BY created_at) AS win_val
FROM smelt.ref('py_l3_441')
