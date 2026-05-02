---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    event_date,
    product_id,
    referrer
FROM smelt.models.clicks
WHERE user_id IN (
    SELECT user_id FROM smelt.models.clicks WHERE status = 'active'
)

