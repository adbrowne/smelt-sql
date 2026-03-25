---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    referrer,
    updated_at,
    LAG(amount, 1) OVER (PARTITION BY referrer ORDER BY created_at) AS win_val
FROM smelt.ref('transactions')
