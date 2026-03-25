---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    is_active,
    browser,
    RANK() OVER (PARTITION BY is_active ORDER BY created_at) AS win_val
FROM smelt.ref('sql_l3_94')
