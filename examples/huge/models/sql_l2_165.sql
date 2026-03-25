---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    browser,
    transaction_id,
    channel,
    is_active
FROM smelt.ref('py_l1_369')
WHERE status = 'active'
