-- Customer order summary with window functions
SELECT
    customer_id,
    customer_segment,
    customer_country,
    COUNT(*) AS order_count,
    SUM(gross_revenue) AS total_revenue,
    SUM(net_revenue) AS total_net_revenue,
    MIN(order_date) AS first_order_date,
    MAX(order_date) AS last_order_date,
    ROW_NUMBER() OVER (
        PARTITION BY customer_country
        ORDER BY SUM(gross_revenue) DESC
    ) AS country_revenue_rank
FROM smelt.ref('int_order_enriched')
GROUP BY customer_id, customer_segment, customer_country
HAVING COUNT(*) >= 1
