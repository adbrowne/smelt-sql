---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    score,
    device_type,
    ROW_NUMBER() OVER (PARTITION BY score ORDER BY created_at) AS win_val
FROM smelt.ref('sql_l3_155')
