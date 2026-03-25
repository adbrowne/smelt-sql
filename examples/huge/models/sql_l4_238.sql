---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    amount,
    event_date,
    created_at
FROM smelt.ref('sql_l3_190')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('sql_l3_82') WHERE created_at >= '2024-01-01'
)
