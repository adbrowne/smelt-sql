---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    score,
    COUNT(*) AS val_1,
    COUNT(DISTINCT user_id) AS val_2
FROM smelt.ref('sql_l3_93')
GROUP BY score
HAVING COUNT(*) > 10
