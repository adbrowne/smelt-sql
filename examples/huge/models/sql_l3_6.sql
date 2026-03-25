---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    campaign_id,
    product_id,
    tier
FROM smelt.ref('py_l2_341')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('sql_l2_223') WHERE platform = 'web'
)
