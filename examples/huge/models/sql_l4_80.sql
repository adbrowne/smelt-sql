---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT plan_type, email_domain, segment, 'source_0' AS source_tag FROM smelt.ref('sql_l3_36')
UNION ALL
SELECT plan_type, email_domain, segment, 'source_1' AS source_tag FROM smelt.ref('py_l3_305')
UNION ALL
SELECT plan_type, email_domain, segment, 'source_2' AS source_tag FROM smelt.ref('sql_l3_4')
