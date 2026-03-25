---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    event_date,
    user_id,
    tier
FROM smelt.ref('sql_l3_173')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('py_l3_450') WHERE score >= 50
)
