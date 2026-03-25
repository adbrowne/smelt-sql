---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    country,
    ip_address,
    event_date,
    cohort_date
FROM smelt.ref('py_l1_444')
WHERE event_type = 'purchase'
