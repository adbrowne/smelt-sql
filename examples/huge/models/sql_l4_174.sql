---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    event_date,
    amount,
    session_id,
    product_id
FROM smelt.ref('sql_l3_177')
WHERE status = 'active'
