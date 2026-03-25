---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    event_date,
    amount,
    session_id,
    product_id
FROM smelt.ref('sql_l3_187')
WHERE status = 'active'
