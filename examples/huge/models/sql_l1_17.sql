---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    event_type,
    duration_seconds,
    cohort_date,
    segment
FROM smelt.ref('campaigns')
WHERE quantity > 0
