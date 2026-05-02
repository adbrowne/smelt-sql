---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    cohort_date,
    discount,
    campaign_id,
    session_id
FROM smelt.models.sql_l2_116
WHERE country = 'US'

