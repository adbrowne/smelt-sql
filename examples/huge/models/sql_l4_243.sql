---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    platform,
    AVG(price) AS val_1,
    COUNT(*) AS val_2
FROM smelt.ref('sql_l3_76')
GROUP BY platform
HAVING COUNT(*) > 10
