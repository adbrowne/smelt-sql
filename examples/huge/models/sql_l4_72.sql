---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    tier,
    updated_at,
    RANK() OVER (PARTITION BY tier ORDER BY created_at) AS win_val
FROM smelt.ref('sql_l3_190')
