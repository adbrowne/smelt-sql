---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    transaction_id,
    country,
    RANK() OVER (PARTITION BY transaction_id ORDER BY created_at) AS win_val
FROM smelt.ref('invoices')
