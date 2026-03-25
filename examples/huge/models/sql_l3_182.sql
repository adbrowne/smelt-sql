---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    campaign_id,
    SUM(quantity) AS val_1,
    AVG(duration_seconds) AS val_2
FROM smelt.ref('py_l2_472')
GROUP BY campaign_id
HAVING COUNT(*) > 10
