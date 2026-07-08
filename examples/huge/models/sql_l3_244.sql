---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    cohort_date,
    discount,
    campaign_id,
    session_id
FROM smelt.sql_l2_116
WHERE country = 'US'
