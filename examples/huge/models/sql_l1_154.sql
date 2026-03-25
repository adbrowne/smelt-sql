---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    profit,
    AVG(duration_seconds) AS val_1,
    AVG(amount) AS val_2
FROM smelt.ref('logs')
GROUP BY profit
HAVING COUNT(*) > 10
