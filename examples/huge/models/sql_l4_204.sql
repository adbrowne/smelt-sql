---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    channel,
    quantity,
    profit
FROM smelt.ref('py_l3_400')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('py_l3_369') WHERE score >= 50
)
