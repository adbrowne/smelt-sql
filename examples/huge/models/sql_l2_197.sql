---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT rating, country, profit, 'source_0' AS source_tag FROM smelt.models.sql_l1_128
UNION ALL
SELECT rating, country, profit, 'source_1' AS source_tag FROM smelt.models.sql_l1_27
UNION ALL
SELECT rating, country, profit, 'source_2' AS source_tag FROM smelt.models.sql_l1_56

