---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    browser,
    campaign_id,
    amount
FROM smelt.ref('sql_l1_129')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('py_l1_343') WHERE created_at >= '2024-01-01'
)
