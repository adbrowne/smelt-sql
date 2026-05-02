---
materialization: table
target: spark_docker
---
SELECT
    visitor_id,
    session_id,
    CAST(session_date AS TIMESTAMP) AS session_start,
    CAST(session_date AS DATE) AS session_date,
    page_views,
    COALESCE(revenue_cents, 0) AS revenue_cents,
    country,
    traffic_source,
    device_type,
    is_converted
FROM smelt.sources.raw.sessions
WHERE visitor_id IS NOT NULL

