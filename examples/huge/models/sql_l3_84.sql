---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    score,
    country,
    campaign_id
FROM smelt.ref('py_l2_409')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('py_l2_498') WHERE status = 'active'
)
