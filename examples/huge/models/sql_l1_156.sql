---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    channel,
    country,
    LAG(amount, 1) OVER (PARTITION BY channel ORDER BY created_at) AS win_val
FROM smelt.ref('signups')
