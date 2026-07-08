---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    product_id,
    score,
    page_path
FROM smelt.subscriptions
WHERE user_id IN (
    SELECT user_id FROM smelt.subscriptions WHERE country = 'US'
)
