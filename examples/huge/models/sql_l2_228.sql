---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    cohort_date,
    device_type,
    email_domain
FROM smelt.ref('sql_l1_55')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('py_l1_320') WHERE amount > 0
)
