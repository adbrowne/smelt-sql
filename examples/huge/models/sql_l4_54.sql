---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    region,
    browser,
    ROW_NUMBER() OVER (PARTITION BY region ORDER BY created_at) AS win_val
FROM smelt.ref('sql_l3_28')
