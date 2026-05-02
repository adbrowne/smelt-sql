-- Funnel conversion metrics (bugs #3 CASE in aggregates, #7 DECIMAL division)
SELECT
    visitor_id,
    COUNT(*) AS total_events,
    COUNT(CASE WHEN event_type = 'page_view' THEN 1 END) AS page_views,
    COUNT(CASE WHEN event_type = 'add_to_cart' THEN 1 END) AS cart_adds,
    COUNT(CASE WHEN event_type = 'purchase' THEN 1 END) AS purchases,
    COUNT(CASE WHEN event_type = 'purchase' THEN 1 END) * 1.0 / COUNT(*) AS conversion_rate
FROM smelt.models.staging.stg_events
GROUP BY visitor_id

