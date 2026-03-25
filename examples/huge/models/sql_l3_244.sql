---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    cohort_date,
    discount,
    campaign_id,
    session_id
FROM smelt.ref('sql_l2_193')
WHERE country = 'US'
