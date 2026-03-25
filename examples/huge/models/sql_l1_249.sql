---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    status,
    AVG(price) AS agg_0,
    COUNT(*) AS agg_1,
    AVG(amount) AS agg_2
FROM smelt.ref('categories')
GROUP BY status
