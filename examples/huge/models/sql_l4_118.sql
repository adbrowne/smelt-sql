---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    page_path,
    MAX(created_at) AS val_1,
    SUM(revenue) AS val_2
FROM smelt.ref('sql_l3_136')
GROUP BY page_path
HAVING COUNT(*) > 10
