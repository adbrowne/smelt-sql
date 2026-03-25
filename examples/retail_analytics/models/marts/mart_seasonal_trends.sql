-- Seasonal trends with CTE + window function and GROUP BY expression
WITH daily_metrics AS (
    SELECT
        order_date,
        DAYOFWEEK(order_date) AS day_of_week,
        MONTH(order_date) AS month,
        COUNT(DISTINCT order_id) AS order_count,
        SUM(net_revenue) AS daily_revenue
    FROM smelt.ref('int_order_enriched')
    GROUP BY order_date
),

weekly_summary AS (
    SELECT
        day_of_week,
        month,
        SUM(order_count) AS total_orders,
        SUM(daily_revenue) AS total_revenue,
        AVG(daily_revenue) AS avg_daily_revenue
    FROM daily_metrics
    GROUP BY day_of_week, month
)

SELECT
    day_of_week,
    month,
    total_orders,
    total_revenue,
    avg_daily_revenue,
    AVG(total_revenue) OVER (
        PARTITION BY day_of_week
    ) AS day_of_week_avg_revenue
FROM weekly_summary
