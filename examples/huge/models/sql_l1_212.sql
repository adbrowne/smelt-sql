---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    cohort_date,
    channel,
    price
FROM smelt.ref('errors')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('errors') WHERE event_type = 'purchase'
)
