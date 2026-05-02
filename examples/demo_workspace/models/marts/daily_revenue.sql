-- Daily revenue rollup from events
SELECT
    DATE_TRUNC('day', event_time) AS revenue_date,
    COUNT(DISTINCT user_id) AS paying_users,
    SUM(revenue_cents) AS total_revenue_cents
FROM smelt.models.staging.stg_events
WHERE event_type = 'purchase'
GROUP BY revenue_date

