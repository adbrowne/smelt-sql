---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    country,
    segment,
    discount
FROM smelt.models.payments
WHERE user_id IN (
    SELECT user_id FROM smelt.models.payments WHERE platform = 'web'
)

