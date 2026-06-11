---
materialization: table
target: duckdb_local
incremental:
  enabled: true
timeseries:
  event_time_column: session_date
  partition_column: metric_date
  granularity: day
---
SELECT
    session_date AS metric_date,
    COUNT(DISTINCT visitor_id) AS unique_visitors,
    SUM(session_count) AS total_sessions,
    SUM(total_page_views) AS total_page_views,
    SUM(total_revenue_cents) AS total_revenue_cents,
    ROUND(CAST(SUM(total_revenue_cents) AS DOUBLE) / 100.0, 2) AS total_revenue,
    SUM(conversions) AS total_conversions,
    ROUND(CAST(SUM(conversions) AS DOUBLE) / COUNT(DISTINCT visitor_id) * 100, 2) AS conversion_rate_pct,
    ROUND(CAST(SUM(total_page_views) AS DOUBLE) / SUM(session_count), 1) AS avg_pages_per_session,
    ROUND(CAST(SUM(total_revenue_cents) AS DOUBLE) / 100.0 / NULLIF(SUM(conversions), 0), 2) AS avg_order_value
FROM smelt.intermediate.int_visitor_daily
GROUP BY session_date

