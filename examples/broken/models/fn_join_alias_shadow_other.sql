-- Companion for `fn_join_alias_shadow.sql`.  Provides both the
-- `orders` source (`order_id`) and the joined `y` model (`id`, `name`)
-- — the `name` column collides with the function's `name` parameter,
-- triggering Phase 15's ParameterShadowsColumn warning via the
-- Phase 45 join-alias visibility extension.
SELECT
  CAST(NULL AS BIGINT) AS order_id,
  CAST(NULL AS BIGINT) AS id,
  CAST(NULL AS VARCHAR) AS name
