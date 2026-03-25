---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    cost,
    segment,
    price,
    score
FROM smelt.ref('sql_l3_240')
WHERE score >= 50
