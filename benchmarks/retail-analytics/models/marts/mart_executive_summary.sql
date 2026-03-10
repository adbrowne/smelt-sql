-- Executive summary using UNION ALL and scalar subqueries
SELECT
    'Total Revenue' AS metric_name,
    CAST(SUM(net_revenue) AS VARCHAR) AS metric_value,
    'currency' AS metric_type
FROM smelt.ref('int_order_enriched')
WHERE order_status = 'fulfilled'

UNION ALL

SELECT
    'Total Orders' AS metric_name,
    CAST(COUNT(DISTINCT order_id) AS VARCHAR) AS metric_value,
    'count' AS metric_type
FROM smelt.ref('int_order_enriched')

UNION ALL

SELECT
    'Unique Customers' AS metric_name,
    CAST(COUNT(DISTINCT customer_id) AS VARCHAR) AS metric_value,
    'count' AS metric_type
FROM smelt.ref('int_order_enriched')

UNION ALL

SELECT
    'Avg Order Value' AS metric_name,
    CAST(SUM(net_revenue) / COUNT(DISTINCT order_id) AS VARCHAR) AS metric_value,
    'currency' AS metric_type
FROM smelt.ref('int_order_enriched')
WHERE order_status = 'fulfilled'
