---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    page_path,
    AVG(amount) AS val_1,
    COUNT(DISTINCT user_id) AS val_2
FROM smelt.ref('sql_l1_50')
GROUP BY page_path
HAVING COUNT(*) > 10
