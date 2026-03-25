---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    discount,
    AVG(amount) AS val_1,
    SUM(revenue) AS val_2
FROM smelt.ref('page_views')
GROUP BY discount
HAVING COUNT(*) > 10
