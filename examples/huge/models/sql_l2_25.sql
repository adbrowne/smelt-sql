---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    device_type,
    cohort_date,
    created_at
FROM smelt.ref('sql_l1_134')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('sql_l1_195') WHERE status = 'active'
)
