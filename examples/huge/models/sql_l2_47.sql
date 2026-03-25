---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    created_at,
    cohort_date,
    browser,
    transaction_id
FROM smelt.ref('py_l1_337')
WHERE status = 'active'
