---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    event_date,
    referrer,
    RANK() OVER (PARTITION BY event_date ORDER BY created_at) AS win_val
FROM smelt.ref('sql_l2_178')
