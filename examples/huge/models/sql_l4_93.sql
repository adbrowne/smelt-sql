---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.cohort_date,
    a.is_verified,
    b.plan_type
FROM smelt.ref('sql_l3_148') a
LEFT JOIN smelt.ref('sql_l3_196') b ON a.user_id = b.user_id
