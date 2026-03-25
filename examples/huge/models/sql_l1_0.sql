---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    country,
    segment,
    discount
FROM smelt.ref('payments')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('payments') WHERE platform = 'web'
)
