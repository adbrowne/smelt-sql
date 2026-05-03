-- Session aggregation using CTEs (bug #5 CTE type inference)
WITH session_bounds AS (
    SELECT
        event_id,
        visitor_id,
        event_type,
        event_timestamp,
        gap_seconds,
        SUM(CASE WHEN gap_seconds IS NULL OR gap_seconds > 1800 THEN 1 ELSE 0 END)
            OVER (PARTITION BY visitor_id ORDER BY event_timestamp) AS session_id
    FROM smelt.staging.stg_events
)

SELECT
    visitor_id,
    session_id,
    MIN(event_timestamp) AS session_start,
    MAX(event_timestamp) AS session_end,
    COUNT(*) AS event_count
FROM session_bounds
GROUP BY visitor_id, session_id

