---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT amount, profit, cohort_date, 'source_0' AS source_tag FROM smelt.ref('errors')
UNION ALL
SELECT amount, profit, cohort_date, 'source_1' AS source_tag FROM smelt.ref('errors')
