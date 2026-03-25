---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    created_at,
    MAX(created_at) AS val_1,
    SUM(revenue) AS val_2
FROM smelt.ref('sql_l2_81')
GROUP BY created_at
HAVING COUNT(*) > 10
