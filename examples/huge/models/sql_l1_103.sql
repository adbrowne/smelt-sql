---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    discount,
    AVG(amount) AS val_1,
    SUM(revenue) AS val_2
FROM smelt.models.page_views
GROUP BY discount
HAVING COUNT(*) > 10

