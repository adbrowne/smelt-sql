---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    order_id,
    segment,
    event_time
FROM smelt.ref('sql_l3_246')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('sql_l3_246') WHERE event_type = 'purchase'
)
