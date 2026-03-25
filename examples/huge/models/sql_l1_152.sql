---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    is_verified,
    transaction_id,
    score,
    rating
FROM smelt.ref('categories')
WHERE status = 'active'
