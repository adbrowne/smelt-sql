-- Staged web events with type normalization
SELECT
    event_id,
    customer_id,
    CASE
        WHEN event_type = 'page_view' THEN 'view'
        WHEN event_type = 'product_view' THEN 'product_view'
        WHEN event_type = 'add_to_cart' THEN 'cart_add'
        WHEN event_type = 'search' THEN 'search'
        WHEN event_type = 'checkout_start' THEN 'checkout'
        WHEN event_type = 'purchase' THEN 'purchase'
        WHEN event_type = 'wishlist_add' THEN 'wishlist'
        ELSE 'other'
    END AS event_type,
    COALESCE(page_category, 'unknown') AS page_category,
    COALESCE(referrer_source, 'direct') AS referrer_source,
    COALESCE(device_type, 'unknown') AS device_type,
    CAST(event_date AS DATE) AS event_date
FROM smelt.sources.raw.web_events

