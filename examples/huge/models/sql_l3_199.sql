---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    is_verified,
    channel,
    os_name
FROM smelt.ref('py_l2_451')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('py_l2_267') WHERE is_active = true
)
