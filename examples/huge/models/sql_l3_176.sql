---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    is_verified,
    event_type,
    cohort_date,
    rating
FROM smelt.ref('sql_l2_221')
WHERE status = 'active'
