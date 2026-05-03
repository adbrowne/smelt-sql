-- Phase 17 broken fixture: caller projects a column that doesn't
-- exist on the inferred TableExpr return schema of a
-- `smelt.<name>(...)` call in FROM position. The caller writes
-- `SELECT missing_col FROM smelt.add_margin_p17(...)`, but the
-- `add_margin_p17` body only projects `{source.*, margin}` so
-- `missing_col` doesn't resolve against the inferred FROM-scope
-- entry — surfaces as UndeclaredColumn at the offending column.
--
-- Companion: `fn_tableexpr_return_bare_col_missing_other.sql`
-- declares the `p17_orders` model that feeds into `add_margin_p17`.

smelt.define add_margin_p17(source: TableExpr) -> TableExpr AS (
  SELECT source.*, revenue - cost AS margin FROM source
)

SELECT missing_col
FROM smelt.add_margin_p17(
  smelt.fn_tableexpr_return_bare_col_missing_other
) AS m
