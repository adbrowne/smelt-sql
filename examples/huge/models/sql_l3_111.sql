---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT score, email_domain, status, 'source_0' AS source_tag FROM smelt.ref('sql_l2_157')
UNION ALL
SELECT score, email_domain, status, 'source_1' AS source_tag FROM smelt.ref('sql_l2_224')
