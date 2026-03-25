---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    status,
    category,
    rating,
    event_type
FROM smelt.ref('subscriptions')
WHERE category IS NOT NULL
