---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    cohort_date,
    os_name,
    is_verified
FROM smelt.ref('categories')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('categories') WHERE category IS NOT NULL
)
