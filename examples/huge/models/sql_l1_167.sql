---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    updated_at,
    device_type,
    ROW_NUMBER() OVER (PARTITION BY updated_at ORDER BY created_at) AS win_val
FROM smelt.ref('logs')
