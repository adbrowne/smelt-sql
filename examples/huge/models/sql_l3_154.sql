---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    is_verified,
    transaction_id,
    RANK() OVER (PARTITION BY is_verified ORDER BY created_at) AS win_val
FROM smelt.ref('sql_l2_5')
