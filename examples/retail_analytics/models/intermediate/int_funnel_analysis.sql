-- Web funnel analysis with conditional aggregation
SELECT
    event_date,
    referrer_source,
    device_type,
    COUNT(*) AS total_events,
    COUNT(CASE WHEN event_type = 'view' THEN 1 END) AS page_views,
    COUNT(CASE WHEN event_type = 'product_view' THEN 1 END) AS product_views,
    COUNT(CASE WHEN event_type = 'cart_add' THEN 1 END) AS cart_adds,
    COUNT(CASE WHEN event_type = 'checkout' THEN 1 END) AS checkouts,
    COUNT(CASE WHEN event_type = 'purchase' THEN 1 END) AS purchases,
    COUNT(CASE WHEN event_type = 'search' THEN 1 END) AS searches,
    COUNT(CASE WHEN event_type = 'wishlist' THEN 1 END) AS wishlist_adds,
    CASE
        WHEN COUNT(CASE WHEN event_type = 'view' THEN 1 END) > 0
        THEN CAST(COUNT(CASE WHEN event_type = 'purchase' THEN 1 END) AS FLOAT)
             / CAST(COUNT(CASE WHEN event_type = 'view' THEN 1 END) AS FLOAT)
        ELSE 0.0
    END AS view_to_purchase_rate
FROM smelt.staging.stg_web_events
GROUP BY event_date, referrer_source, device_type

