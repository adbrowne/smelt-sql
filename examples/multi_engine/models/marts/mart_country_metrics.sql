---
materialization: table
target: duckdb_local
---
SELECT
    country,
    COUNT(DISTINCT visitor_id) AS unique_visitors,
    SUM(session_count) AS total_sessions,
    SUM(total_page_views) AS total_page_views,
    ROUND(CAST(SUM(total_revenue_cents) AS DOUBLE) / 100.0, 2) AS total_revenue,
    SUM(conversions) AS total_conversions,
    ROUND(CAST(SUM(conversions) AS DOUBLE) / COUNT(DISTINCT visitor_id) * 100, 2) AS conversion_rate_pct
FROM smelt.intermediate.int_visitor_daily
GROUP BY country
ORDER BY total_revenue DESC

