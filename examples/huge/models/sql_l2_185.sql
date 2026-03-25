---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    plan_type,
    duration_seconds,
    profit,
    updated_at
FROM smelt.ref('sql_l1_78')
WHERE category IS NOT NULL
