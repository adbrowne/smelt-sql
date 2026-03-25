---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    channel,
    AVG(amount) AS val_1,
    COUNT(*) AS val_2
FROM smelt.ref('sql_l2_103')
GROUP BY channel
HAVING COUNT(*) > 10
