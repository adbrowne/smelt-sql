---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    plan_type,
    AVG(duration_seconds) AS val_1,
    COUNT(*) AS val_2
FROM smelt.ref('sql_l3_172')
GROUP BY plan_type
HAVING COUNT(*) > 10
