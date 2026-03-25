---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    product_id,
    score,
    page_path
FROM smelt.ref('subscriptions')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('subscriptions') WHERE country = 'US'
)
