---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    event_type,
    browser,
    amount
FROM smelt.ref('sql_l2_76')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('py_l2_265') WHERE category IS NOT NULL
)
