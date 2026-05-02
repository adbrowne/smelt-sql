-- Sales ranking with RANK OVER PARTITION BY and NULLS LAST
SELECT
    store_id,
    store_type,
    store_region,
    order_date,
    gross_revenue,
    net_revenue,
    order_count,
    RANK() OVER (
        PARTITION BY store_region, order_date
        ORDER BY gross_revenue DESC NULLS LAST
    ) AS daily_region_rank,
    RANK() OVER (
        PARTITION BY store_type
        ORDER BY SUM(gross_revenue) OVER (
            PARTITION BY store_id
        ) DESC NULLS LAST
    ) AS type_total_rank,
    SUM(gross_revenue) OVER (
        PARTITION BY store_id
        ORDER BY order_date
        ROWS BETWEEN 6 PRECEDING AND CURRENT ROW
    ) AS rolling_7day_revenue
FROM smelt.models.intermediate.int_daily_store_metrics

