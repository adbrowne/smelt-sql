-- Channel attribution with subquery in FROM
SELECT
    referrer_source,
    device_type,
    total_events,
    purchases,
    CASE
        WHEN total_events > 0
        THEN CAST(purchases AS FLOAT) / CAST(total_events AS FLOAT)
        ELSE 0.0
    END AS conversion_rate,
    revenue_attributed
FROM (
    SELECT
        f.referrer_source,
        f.device_type,
        SUM(f.total_events) AS total_events,
        SUM(f.purchases) AS purchases,
        SUM(f.page_views) AS page_views,
        SUM(f.cart_adds) AS cart_adds,
        SUM(f.checkouts) AS checkouts
    FROM smelt.ref('int_funnel_analysis') AS f
    GROUP BY f.referrer_source, f.device_type
) AS channel_stats
LEFT JOIN (
    SELECT
        w.referrer_source,
        w.device_type,
        SUM(o.net_revenue) AS revenue_attributed
    FROM smelt.ref('stg_web_events') AS w
    INNER JOIN smelt.ref('int_order_enriched') AS o
        ON w.customer_id = o.customer_id
        AND w.event_date = o.order_date
    WHERE w.event_type = 'purchase'
    GROUP BY w.referrer_source, w.device_type
) AS revenue_stats
    ON channel_stats.referrer_source = revenue_stats.referrer_source
    AND channel_stats.device_type = revenue_stats.device_type
