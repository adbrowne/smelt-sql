---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    updated_at,
    status,
    ROW_NUMBER() OVER (PARTITION BY updated_at ORDER BY created_at) AS win_val
FROM smelt.ref('refunds')
