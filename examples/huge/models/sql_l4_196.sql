---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    discount,
    score,
    ROW_NUMBER() OVER (PARTITION BY discount ORDER BY created_at) AS win_val
FROM smelt.ref('sql_l3_24')
