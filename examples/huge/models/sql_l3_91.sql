---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    revenue,
    cohort_date,
    event_date
FROM smelt.ref('py_l2_380')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('sql_l2_92') WHERE status = 'active'
)
