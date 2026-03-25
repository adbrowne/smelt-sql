---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    event_type,
    cohort_date,
    category,
    referrer
FROM smelt.ref('orders')
WHERE is_active = true
