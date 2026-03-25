---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    session_id,
    AVG(price) AS val_1,
    AVG(amount) AS val_2
FROM smelt.ref('sql_l1_110')
GROUP BY session_id
HAVING COUNT(*) > 10
