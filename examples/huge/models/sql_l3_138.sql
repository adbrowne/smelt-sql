---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    browser,
    revenue,
    duration_seconds,
    device_type
FROM smelt.ref('py_l2_386')
WHERE status = 'active'
