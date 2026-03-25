---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    profit,
    campaign_id,
    cohort_date,
    status
FROM smelt.ref('errors')
WHERE quantity > 0
