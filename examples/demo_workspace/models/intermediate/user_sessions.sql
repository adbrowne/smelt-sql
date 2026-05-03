-- Sessionized events grouped by user and day
SELECT
    user_id,
    DATE_TRUNC('day', event_time) AS session_date,
    COUNT(*) AS event_count,
    SUM(revenue_cents) AS session_revenue_cents
FROM smelt.staging.stg_events
GROUP BY user_id, session_date

