---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    profit,
    country,
    tier
FROM smelt.ref('py_l1_250')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('sql_l1_43') WHERE score >= 50
)
