---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    tier,
    AVG(duration_seconds) AS val_1,
    SUM(amount) AS val_2
FROM smelt.ref('sql_l1_241')
GROUP BY tier
HAVING COUNT(*) > 10
