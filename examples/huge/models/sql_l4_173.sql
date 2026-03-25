---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    order_id,
    AVG(amount) AS val_1,
    MIN(created_at) AS val_2
FROM smelt.ref('sql_l3_159')
GROUP BY order_id
HAVING COUNT(*) > 10
