---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    status,
    page_path,
    created_at
FROM smelt.ref('categories')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('categories') WHERE quantity > 0
)
