---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    status,
    cost,
    campaign_id
FROM smelt.ref('sql_l1_211')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('py_l1_435') WHERE country = 'US'
)
