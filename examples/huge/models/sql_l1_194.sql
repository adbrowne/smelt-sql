---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT duration_seconds, cost, score, 'source_0' AS source_tag FROM smelt.ref('campaigns')
UNION ALL
SELECT duration_seconds, cost, score, 'source_1' AS source_tag FROM smelt.ref('campaigns')
