-- Anomaly detection using AVG OVER rolling frame and ratio filtering
WITH daily_revenue AS (
    SELECT
        order_date,
        store_id,
        store_type,
        SUM(gross_revenue) AS daily_gross,
        SUM(net_revenue) AS daily_net,
        COUNT(DISTINCT order_id) AS daily_orders
    FROM smelt.ref('int_daily_store_metrics')
    GROUP BY order_date, store_id, store_type
),

store_baselines AS (
    SELECT
        store_id,
        order_date,
        daily_gross,
        daily_orders,
        AVG(daily_gross) OVER (
            PARTITION BY store_id
            ORDER BY order_date
            ROWS BETWEEN 13 PRECEDING AND CURRENT ROW
        ) AS rolling_avg_revenue,
        AVG(daily_orders) OVER (
            PARTITION BY store_id
            ORDER BY order_date
            ROWS BETWEEN 13 PRECEDING AND CURRENT ROW
        ) AS rolling_avg_orders
    FROM daily_revenue
)

SELECT
    sb.store_id,
    sb.order_date,
    sb.daily_gross,
    sb.daily_orders,
    sb.rolling_avg_revenue,
    sb.rolling_avg_orders,
    sb.daily_gross / sb.rolling_avg_revenue AS revenue_ratio,
    sb.daily_orders / sb.rolling_avg_orders AS orders_ratio
FROM store_baselines AS sb
WHERE sb.rolling_avg_revenue > 0
