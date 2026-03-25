---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    referrer,
    cohort_date,
    duration_seconds,
    device_type
FROM smelt.ref('sql_l3_215')
WHERE country = 'US'
