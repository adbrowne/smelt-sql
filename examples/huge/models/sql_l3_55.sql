---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    device_type,
    session_id,
    ROW_NUMBER() OVER (PARTITION BY device_type ORDER BY created_at) AS win_val
FROM smelt.ref('py_l2_258')
