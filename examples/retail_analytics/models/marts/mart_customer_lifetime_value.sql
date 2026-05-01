-- Customer lifetime value with cumulative windows and CTEs
WITH order_history AS (
    SELECT
        customer_id,
        customer_segment,
        customer_country,
        order_date,
        net_revenue,
        SUM(net_revenue) OVER (
            PARTITION BY customer_id
            ORDER BY order_date
            ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW
        ) AS cumulative_revenue,
        ROW_NUMBER() OVER (
            PARTITION BY customer_id
            ORDER BY order_date
        ) AS order_sequence
    FROM smelt.models.intermediate.int_order_enriched
    WHERE order_status = 'fulfilled'
),

customer_summary AS (
    SELECT
        customer_id,
        customer_segment,
        customer_country,
        COUNT(*) AS total_orders,
        SUM(net_revenue) AS lifetime_value,
        MIN(order_date) AS first_order,
        MAX(order_date) AS last_order
    FROM order_history
    GROUP BY customer_id, customer_segment, customer_country
)

SELECT
    cs.customer_id,
    cs.customer_segment,
    cs.customer_country,
    cs.total_orders,
    cs.lifetime_value,
    cs.first_order,
    cs.last_order,
    cs.lifetime_value / cs.total_orders AS avg_order_value
FROM customer_summary AS cs

