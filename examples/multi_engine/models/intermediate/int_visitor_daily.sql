---
materialization: table
target: spark_docker
refresh: incremental
grain: partition
timeseries:
  event_time_column: session_start
  partition_column: session_date
  granularity: day
---
SELECT
    visitor_id,
    session_date,
    country,
    COUNT(*) AS session_count,
    SUM(page_views) AS total_page_views,
    SUM(revenue_cents) AS total_revenue_cents,
    SUM(CASE WHEN is_converted THEN 1 ELSE 0 END) AS conversions,
    MIN(session_start) AS first_session,
    MAX(session_start) AS last_session,
    COUNT(DISTINCT traffic_source) AS traffic_source_count,
    COUNT(DISTINCT device_type) AS device_count
FROM smelt.staging.stg_sessions
GROUP BY visitor_id, session_date, country

