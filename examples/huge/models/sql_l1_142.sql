---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    event_date,
    COUNT(*) AS val_1,
    AVG(price) AS val_2
FROM smelt.ref('sessions')
GROUP BY event_date
HAVING COUNT(*) > 10
