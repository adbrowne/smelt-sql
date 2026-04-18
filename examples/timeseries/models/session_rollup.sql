-- CUBE rollup: GROUPING() distinguishes "this column was rolled up by CUBE"
-- from "this column happened to be NULL in the source row". Using
-- COALESCE(col, 'all') here would collapse real NULLs and rollup NULLs into
-- the same row, producing duplicate "grand total" rows. See
-- docs/reference/language.md#labelling-rollup-rows.
SELECT
  COUNT(DISTINCT session_id) AS session_count,
  COUNT(DISTINCT visitor_id) AS visitor_count,
  SUM(product_views) AS total_product_views,
  SUM(widget_views) AS total_widget_views,
  SUM(product_revenue) AS total_product_revenue,
  CASE WHEN GROUPING(visit_source)     = 1 THEN 'all' ELSE visit_source     END AS traffic_source,
  CASE WHEN GROUPING(platform)         = 1 THEN 'all' ELSE platform         END AS platform,
  CASE WHEN GROUPING(visit_campaign)   = 1 THEN 'all' ELSE visit_campaign   END AS visit_campaign,
  CASE WHEN GROUPING(product_category) = 1 THEN 'all' ELSE product_category END AS product_category
FROM smelt.source('raw.sessions') AS sessions
GROUP BY CUBE(
  sessions.visit_source,
  sessions.platform,
  sessions.visit_campaign,
  sessions.product_category,
  sessions.foo_bar
)
