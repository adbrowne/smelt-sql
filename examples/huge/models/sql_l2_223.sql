---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    channel,
    SUM(amount) AS val_1,
    AVG(duration_seconds) AS val_2
FROM smelt.ref('sql_l1_168')
GROUP BY channel
HAVING COUNT(*) > 10
