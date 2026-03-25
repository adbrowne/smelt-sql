---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    status,
    page_path,
    created_at
FROM smelt.ref('categories')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('categories') WHERE quantity > 0
)
