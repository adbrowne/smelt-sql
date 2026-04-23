-- Phase 15 fixture: call a `TableExpr`-taking function with a caller
-- whose schema lacks the `revenue` column. The body check runs at
-- call-site expansion, so the bare `revenue` reference inside
-- `add_margin_local` produces an `UnknownIdentifier` diagnostic with a
-- frame rooted at the call site.
--
-- The companion `fn_tableexpr_missing_col_other.sql` provides the
-- upstream model (`orders_no_revenue`) that lacks the `revenue` column.

smelt.define add_margin_local(source: TableExpr) -> TableExpr AS (
  SELECT source.*, revenue - cost AS margin FROM source
)

SELECT *
FROM smelt.fn.add_margin_local(
  smelt.ref('fn_tableexpr_missing_col_other')
) AS m
