---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    browser,
    campaign_id,
    session_id
FROM smelt.ref('py_l1_486')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('py_l1_486') WHERE country = 'US'
)
