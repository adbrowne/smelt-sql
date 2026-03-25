---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    is_verified,
    status,
    RANK() OVER (PARTITION BY is_verified ORDER BY created_at) AS win_val
FROM smelt.ref('reviews')
