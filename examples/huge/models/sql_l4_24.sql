---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    status,
    referrer,
    RANK() OVER (PARTITION BY status ORDER BY created_at) AS win_val
FROM smelt.ref('sql_l3_81')
