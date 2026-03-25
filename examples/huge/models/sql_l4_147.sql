---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    channel,
    quantity,
    ROW_NUMBER() OVER (PARTITION BY channel ORDER BY created_at) AS win_val
FROM smelt.ref('sql_l3_229')
