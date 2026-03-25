---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    product_id,
    score,
    page_path
FROM smelt.ref('subscriptions')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('subscriptions') WHERE country = 'US'
)
