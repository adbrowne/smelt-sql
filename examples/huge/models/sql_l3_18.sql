---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    category,
    browser,
    RANK() OVER (PARTITION BY category ORDER BY created_at) AS win_val
FROM smelt.ref('sql_l2_185')
