---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    score,
    profit,
    segment,
    tier
FROM smelt.ref('sql_l1_65')
WHERE platform = 'web'
