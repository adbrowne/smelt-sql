-- Companion for `fn_join_alias_missing_col.sql`.  Acts as both the
-- `orders` source (providing `order_id`) and the joined `dim_y` model
-- (providing `id` plus `name` — but deliberately no `does_not_exist`,
-- so the body's `y.does_not_exist` reference triggers the
-- UnknownIdentifier diagnostic via the join-alias path).
SELECT
  CAST(NULL AS BIGINT) AS order_id,
  CAST(NULL AS BIGINT) AS id,
  CAST(NULL AS VARCHAR) AS name
