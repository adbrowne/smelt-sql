---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    device_type,
    tier,
    campaign_id
FROM smelt.ref('sql_l2_186')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('py_l2_479') WHERE country = 'US'
)
