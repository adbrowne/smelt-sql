---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    os_name,
    AVG(amount) AS agg_0,
    COUNT(*) AS agg_1
FROM smelt.ref('shipments')
GROUP BY os_name
