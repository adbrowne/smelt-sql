---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    profit,
    MAX(created_at) AS val_1,
    SUM(amount) AS val_2
FROM smelt.ref('sql_l2_209')
GROUP BY profit
HAVING COUNT(*) > 10
