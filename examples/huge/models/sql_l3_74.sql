---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    is_active,
    category,
    page_path,
    created_at
FROM smelt.ref('sql_l2_54')
WHERE platform = 'web'
