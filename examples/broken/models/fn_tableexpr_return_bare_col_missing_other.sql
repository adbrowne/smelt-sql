-- Companion for `fn_tableexpr_return_bare_col_missing.sql`. Provides
-- the `p17_orders`-style model that feeds into `add_margin_p17`. The
-- revenue / cost columns let the body check typecheck cleanly so
-- the only diagnostic to surface is the outer SELECT's projection
-- of `missing_col` from the inferred return schema.
SELECT
  CAST(NULL AS BIGINT) AS order_id,
  CAST(NULL AS DECIMAL(18, 2)) AS revenue,
  CAST(NULL AS DECIMAL(18, 2)) AS cost
