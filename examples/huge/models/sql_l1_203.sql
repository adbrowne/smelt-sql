---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    revenue,
    updated_at,
    discount
FROM smelt.ref('users')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('users') WHERE event_type = 'purchase'
)
