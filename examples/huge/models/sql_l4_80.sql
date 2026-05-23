---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT plan_type, email_domain, segment, 'source_0' AS source_tag FROM smelt.sql_l3_173
UNION ALL
SELECT plan_type, email_domain, segment, 'source_1' AS source_tag FROM smelt.sql_l3_66
UNION ALL
SELECT plan_type, email_domain, segment, 'source_2' AS source_tag FROM smelt.sql_l3_165
