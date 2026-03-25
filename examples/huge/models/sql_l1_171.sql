---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    status,
    country,
    event_type,
    plan_type
FROM smelt.ref('sessions')
WHERE score >= 50
