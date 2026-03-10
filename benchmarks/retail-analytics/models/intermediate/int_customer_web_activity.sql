-- Customer web activity summary using BETWEEN, IN, and EXISTS
SELECT
    w.customer_id,
    c.segment AS customer_segment,
    COUNT(*) AS total_events,
    COUNT(DISTINCT w.event_date) AS active_days,
    COUNT(CASE WHEN w.event_type IN ('purchase', 'checkout', 'cart_add') THEN 1 END) AS conversion_events,
    COUNT(CASE WHEN w.device_type = 'mobile' THEN 1 END) AS mobile_events,
    COUNT(CASE WHEN w.device_type = 'desktop' THEN 1 END) AS desktop_events
FROM smelt.ref('stg_web_events') AS w
INNER JOIN smelt.ref('stg_customers') AS c ON w.customer_id = c.customer_id
WHERE w.event_date BETWEEN CAST('2024-07-01' AS DATE) AND CAST('2024-09-28' AS DATE)
GROUP BY w.customer_id, c.segment
