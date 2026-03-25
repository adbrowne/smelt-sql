---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    rating,
    channel,
    plan_type,
    user_id
FROM smelt.ref('py_l3_323')
WHERE score >= 50
