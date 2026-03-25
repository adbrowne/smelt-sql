---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    discount,
    browser,
    user_id
FROM smelt.ref('orders')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('orders') WHERE country = 'US'
)
