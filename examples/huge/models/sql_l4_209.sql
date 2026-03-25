---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    rating,
    country,
    ROW_NUMBER() OVER (PARTITION BY rating ORDER BY created_at) AS win_val
FROM smelt.ref('py_l3_337')
