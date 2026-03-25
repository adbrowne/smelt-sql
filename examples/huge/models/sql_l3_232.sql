---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    discount,
    platform,
    campaign_id
FROM smelt.ref('py_l2_263')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('py_l2_463') WHERE category IS NOT NULL
)
