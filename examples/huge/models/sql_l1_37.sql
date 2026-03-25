---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    rating,
    duration_seconds,
    RANK() OVER (PARTITION BY rating ORDER BY created_at) AS win_val
FROM smelt.ref('page_views')
