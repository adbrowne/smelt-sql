---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    status,
    quantity,
    ROW_NUMBER() OVER (PARTITION BY status ORDER BY created_at) AS win_val
FROM smelt.ref('sessions')
