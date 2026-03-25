---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    category,
    browser,
    segment,
    updated_at
FROM smelt.ref('sql_l3_148')
WHERE score >= 50
