---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    channel,
    status,
    browser
FROM smelt.ref('sql_l2_107')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('sql_l2_217') WHERE status = 'active'
)
