---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    is_verified,
    page_path,
    region
FROM smelt.ref('sql_l3_17')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('py_l3_498') WHERE platform = 'web'
)
