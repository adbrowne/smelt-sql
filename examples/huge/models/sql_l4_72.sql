---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    tier,
    updated_at,
    RANK() OVER (PARTITION BY tier ORDER BY created_at) AS win_val
FROM smelt.ref('sql_l3_70')
