---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    channel,
    AVG(amount) AS agg_0,
    COUNT(*) AS agg_1
FROM smelt.ref('py_l3_295')
GROUP BY channel
