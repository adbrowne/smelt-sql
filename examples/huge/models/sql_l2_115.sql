---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT page_path, region, cohort_date, 'source_0' AS source_tag FROM smelt.ref('py_l1_267')
UNION ALL
SELECT page_path, region, cohort_date, 'source_1' AS source_tag FROM smelt.ref('py_l1_278')
UNION ALL
SELECT page_path, region, cohort_date, 'source_2' AS source_tag FROM smelt.ref('sql_l1_9')
