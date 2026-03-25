---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    event_time,
    os_name,
    transaction_id
FROM smelt.ref('sql_l1_214')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('sql_l1_83') WHERE country = 'US'
)
