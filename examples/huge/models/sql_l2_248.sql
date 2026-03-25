---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    profit,
    device_type,
    browser,
    campaign_id
FROM smelt.ref('py_l1_394')
WHERE platform = 'web'
