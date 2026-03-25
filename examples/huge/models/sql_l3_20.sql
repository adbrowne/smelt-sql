---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    event_type,
    quantity,
    country
FROM smelt.ref('sql_l2_201')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('sql_l2_75') WHERE event_type = 'purchase'
)
