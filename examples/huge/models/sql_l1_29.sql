---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    transaction_id,
    status,
    LAG(amount, 1) OVER (PARTITION BY transaction_id ORDER BY created_at) AS win_val
FROM smelt.ref('subscriptions')
