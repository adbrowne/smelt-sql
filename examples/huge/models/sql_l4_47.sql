---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    rating,
    cohort_date,
    created_at,
    channel
FROM smelt.ref('py_l3_310')
WHERE country = 'US'
