---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    campaign_id,
    price,
    channel
FROM smelt.ref('py_l2_269')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('py_l2_376') WHERE quantity > 0
)
