---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    updated_at,
    transaction_id,
    RANK() OVER (PARTITION BY updated_at ORDER BY created_at) AS win_val
FROM smelt.ref('py_l3_443')
