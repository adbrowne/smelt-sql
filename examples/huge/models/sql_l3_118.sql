---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    campaign_id,
    MAX(created_at) AS val_1,
    SUM(quantity) AS val_2
FROM smelt.ref('sql_l2_73')
GROUP BY campaign_id
HAVING COUNT(*) > 10
