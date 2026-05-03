-- Geographic analysis with aggregation and ORDER BY expression
SELECT DISTINCT
    c.country,
    c.city,
    COUNT(DISTINCT o.order_id) AS order_count,
    COUNT(DISTINCT o.customer_id) AS customer_count,
    SUM(o.net_revenue) AS total_revenue,
    SUM(o.net_revenue) / COUNT(DISTINCT o.customer_id) AS revenue_per_customer,
    SUM(o.net_revenue) / COUNT(DISTINCT o.order_id) AS avg_order_value
FROM smelt.staging.stg_customers AS c
INNER JOIN smelt.intermediate.int_order_enriched AS o ON c.customer_id = o.customer_id
GROUP BY c.country, c.city
ORDER BY SUM(o.net_revenue) DESC

