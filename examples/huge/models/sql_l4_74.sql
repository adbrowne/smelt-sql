---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    campaign_id,
    event_time,
    category
FROM smelt.ref('py_l3_359')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('py_l3_359') WHERE platform = 'web'
)
