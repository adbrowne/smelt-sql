-- Companion for `fn_tableexpr_missing_col.sql`. Declares an
-- `orders_no_revenue`-shaped upstream model with a `cost` column but
-- deliberately no `revenue`, so the call to `add_margin_local` tripping
-- on `revenue - cost` inside its body emits a call-site
-- UnknownIdentifier.
SELECT
  CAST(NULL AS BIGINT) AS order_id,
  CAST(NULL AS DECIMAL(18, 2)) AS cost
