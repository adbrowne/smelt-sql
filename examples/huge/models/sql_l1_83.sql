---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    session_id,
    cohort_date,
    referrer
FROM smelt.models.shipments
WHERE user_id IN (
    SELECT user_id FROM smelt.models.shipments WHERE status = 'active'
)

