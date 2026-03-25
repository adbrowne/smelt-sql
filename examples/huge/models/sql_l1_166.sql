---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    is_active,
    order_id,
    duration_seconds,
    revenue
FROM smelt.ref('notifications')
WHERE category IS NOT NULL
