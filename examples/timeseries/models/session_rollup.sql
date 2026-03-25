SELECT
  COUNT(DISTINCT session_id) AS session_count,
  COUNT(DISTINCT visitor_id) AS visitor_count,
  SUM(product_views) AS total_product_views,
  SUM(widget_views) AS total_widget_views,
  SUM(product_revenue) AS total_product_revenue,
  COALESCE(visit_source, 'all') AS traffic_source,
  COALESCE(platform,'all') AS platform,
  COALESCE(visit_campaign,'all') AS visit_campaign,
  COALESCE(product_category,'all') AS product_category
FROM smelt.source('raw.sessions') AS sessions
GROUP BY CUBE(
  sessions.visit_source,
  sessions.platform,
  sessions.visit_campaign,
  sessions.product_category
)