-- Phase 16 fixture: call `add_margin_req` (declared with
-- `TableExpr<{revenue: Numeric, cost: Numeric}>`) against an upstream
-- model that is missing the required `cost` column. The row-
-- requirement check fires at the call site *before* body expansion,
-- surfacing a `RowRequirementUnsatisfied` diagnostic naming the
-- missing `cost` column. Because the requirement fails, the body is
-- NOT re-checked — no cascade UnknownIdentifier diagnostics.
--
-- The companion `fn_row_requirement_missing_other.sql` provides the
-- upstream model `row_req_no_cost` that lacks `cost`.

smelt.define add_margin_req(source: TableExpr<{revenue: Numeric, cost: Numeric}>) -> TableExpr AS (
  SELECT source.*, revenue - cost AS margin FROM source
)

SELECT *
FROM smelt.functions.add_margin_req(
  smelt.models.fn_row_requirement_missing_other
) AS m
