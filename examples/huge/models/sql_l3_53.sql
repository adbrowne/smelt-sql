---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    transaction_id,
    browser,
    revenue
FROM smelt.ref('py_l2_264')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('sql_l2_189') WHERE score >= 50
)
