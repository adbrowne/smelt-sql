---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT score, email_domain, status, 'source_0' AS source_tag FROM smelt.sql_l2_69
UNION ALL
SELECT score, email_domain, status, 'source_1' AS source_tag FROM smelt.sql_l2_224
UNION ALL
SELECT score, email_domain, status, 'source_2' AS source_tag FROM smelt.sql_l2_226

