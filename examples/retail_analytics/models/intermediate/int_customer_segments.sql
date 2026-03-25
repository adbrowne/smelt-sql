-- Customer segmentation using CTEs and NTILE
WITH customer_metrics AS (
    SELECT
        customer_id,
        customer_segment,
        order_count,
        total_revenue,
        total_net_revenue
    FROM smelt.ref('int_customer_orders')
),

customer_quantiles AS (
    SELECT
        customer_id,
        customer_segment,
        order_count,
        total_revenue,
        NTILE(10) OVER (ORDER BY total_revenue DESC) AS revenue_decile,
        NTILE(10) OVER (ORDER BY order_count DESC) AS frequency_decile
    FROM customer_metrics
)

SELECT
    customer_id,
    customer_segment,
    order_count,
    total_revenue,
    revenue_decile,
    frequency_decile,
    CASE
        WHEN revenue_decile <= 2 THEN CASE WHEN frequency_decile <= 2 THEN 'Champion' ELSE 'Loyal' END
        WHEN revenue_decile <= 3 THEN 'Loyal'
        WHEN frequency_decile <= 3 THEN 'Frequent'
        WHEN revenue_decile <= 5 THEN 'Promising'
        WHEN revenue_decile <= 7 THEN 'Average'
        ELSE 'At Risk'
    END AS computed_segment
FROM customer_quantiles
