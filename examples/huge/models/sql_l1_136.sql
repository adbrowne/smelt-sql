---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    segment,
    transaction_id,
    ROW_NUMBER() OVER (PARTITION BY segment ORDER BY created_at) AS win_val
FROM smelt.ref('orders')
