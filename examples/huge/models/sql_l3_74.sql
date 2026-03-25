---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    is_active,
    category,
    page_path,
    created_at
FROM smelt.ref('sql_l2_148')
WHERE platform = 'web'
